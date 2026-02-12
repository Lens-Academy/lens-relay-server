use anyhow::{anyhow, Result};
use axum::{
    body::Bytes,
    extract::{
        multipart::Multipart,
        ws::{CloseFrame, Message, WebSocket},
        Path, Query, Request, State, WebSocketUpgrade,
    },
    http::{
        header::{HeaderName, HeaderValue},
        StatusCode,
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, head, post},
    Json, Router,
};
use axum_extra::typed_header::TypedHeader;
use dashmap::{mapref::one::MappedRef, DashMap};
use futures::{SinkExt, StreamExt, TryStreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    sync::{Arc, RwLock},
    time::Duration,
};
use tempfile::NamedTempFile;
use tokio::{
    net::TcpListener,
    sync::mpsc::{channel, Receiver},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::{span, Instrument, Level};
use url::Url;
use yrs::{GetString, Map, ReadTxn, Transact};
use y_sweet_core::{
    api_types::{
        validate_doc_name, validate_file_hash, AuthDocRequest, Authorization, ClientToken,
        DocCreationRequest, DocumentVersionEntry, DocumentVersionResponse, FileDownloadUrlResponse,
        FileHistoryEntry, FileHistoryResponse, FileUploadUrlResponse, NewDocResponse,
    },
    auth::{Authenticator, ExpirationTimeEpochMillis, Permission, DEFAULT_EXPIRATION_SECONDS},
    doc_connection::DocConnection,
    doc_sync::DocWithSyncKv,
    event::{
        DebouncedSyncProtocolEventSender, DocumentUpdatedEvent, EventDispatcher, EventEnvelope,
        EventSender, SyncProtocolEventSender, UnifiedEventDispatcher, WebhookSender,
    },
    doc_resolver::DocumentResolver,
    link_indexer::{self, LinkIndexer},
    metrics::RelayMetrics,
    search_index::SearchIndex,
    store::Store,
    sync::awareness::Awareness,
    sync_kv::SyncKv,
    webhook::WebhookConfig,
};

const RELAY_SERVER_VERSION: &str = env!("GIT_VERSION");

#[derive(Clone, Debug)]
pub struct AllowedHost {
    pub host: String,
    pub scheme: String, // "http" or "https"
}

fn current_time_epoch_millis() -> u64 {
    let now = std::time::SystemTime::now();
    let duration_since_epoch = now.duration_since(std::time::UNIX_EPOCH).unwrap();
    duration_since_epoch.as_millis() as u64
}

fn validate_file_token(
    server_state: &Arc<Server>,
    token: &str,
    doc_id: &str,
) -> Result<Permission, AppError> {
    let authenticator = server_state.authenticator.as_ref().ok_or_else(|| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            anyhow!("No authenticator configured"),
        )
    })?;

    let permission = authenticator
        .verify_token_auto(token, current_time_epoch_millis())
        .map_err(|auth_error| {
            // Record auth failure metric
            server_state.metrics.record_auth_failure(
                auth_error.to_metric_label(),
                "file_access",
                "POST",
            );
            AppError(StatusCode::UNAUTHORIZED, anyhow!("Invalid token"))
        })?;

    match &permission {
        Permission::File(file_permission) => {
            if file_permission.doc_id != doc_id {
                server_state.metrics.record_permission_denied(
                    "file",
                    "access_wrong_document",
                    "file_access",
                );
                return Err(AppError(
                    StatusCode::UNAUTHORIZED,
                    anyhow!("Token not valid for this document"),
                ));
            }
        }
        _ => {
            server_state.metrics.record_permission_denied(
                "file",
                "wrong_token_type",
                "file_access",
            );
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                anyhow!("Token must be a file token"),
            ));
        }
    }

    Ok(permission)
}

#[derive(Debug)]
pub struct AppError(StatusCode, anyhow::Error);
impl std::error::Error for AppError {}
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (self.0, format!("Something went wrong: {}", self.1)).into_response()
    }
}
impl<E> From<(StatusCode, E)> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from((status_code, err): (StatusCode, E)) -> Self {
        Self(status_code, err.into())
    }
}
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Status code: {} {}", self.0, self.1)?;
        Ok(())
    }
}

#[derive(Deserialize)]
struct FileDownloadQueryParams {
    hash: Option<String>,
}

#[derive(Deserialize)]
struct FileUploadParams {
    token: String,
}

#[derive(Deserialize)]
struct FileDownloadParams {
    token: String,
    hash: String,
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default = "default_search_limit")]
    limit: usize,
}

fn default_search_limit() -> usize {
    20
}

// ---------------------------------------------------------------------------
// Search index background worker
// ---------------------------------------------------------------------------

const SEARCH_DEBOUNCE: Duration = Duration::from_secs(2);

/// Background worker for incremental search index updates.
///
/// Follows the same debounce pattern as LinkIndexer::run_worker:
/// - Content docs: debounce 2 seconds, then re-read Y.Text("contents") and upsert
/// - Folder docs: process immediately, detect added/removed docs, update search index
async fn search_worker(
    mut rx: tokio::sync::mpsc::Receiver<String>,
    search_index: Arc<SearchIndex>,
    docs: Arc<DashMap<String, DocWithSyncKv>>,
    pending: Arc<DashMap<String, tokio::time::Instant>>,
) {
    tracing::info!("Search index worker started");

    // Cache of folder doc -> { uuid -> (path, title) } for detecting adds/removes
    let filemeta_cache: DashMap<String, std::collections::HashMap<String, String>> =
        DashMap::new();

    loop {
        match rx.recv().await {
            Some(doc_id) => {
                let folder_content = link_indexer::is_folder_doc(&doc_id, &docs);

                if folder_content.is_none() {
                    // Content doc — debounce: wait until no updates for SEARCH_DEBOUNCE
                    loop {
                        tokio::time::sleep(SEARCH_DEBOUNCE).await;
                        if let Some(entry) = pending.get(&doc_id) {
                            if entry.elapsed() >= SEARCH_DEBOUNCE {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                }

                // Remove from pending
                pending.remove(&doc_id);

                if let Some(content_uuids) =
                    folder_content.or_else(|| link_indexer::is_folder_doc(&doc_id, &docs))
                {
                    // Folder doc — detect added/removed documents
                    search_handle_folder_update(
                        &doc_id,
                        &content_uuids,
                        &docs,
                        &search_index,
                        &filemeta_cache,
                        &pending,
                        &rx,
                    )
                    .await;
                } else {
                    // Content doc — reindex into search
                    search_handle_content_update(&doc_id, &docs, &search_index);
                }
            }
            None => break,
        }
    }
}

/// Handle a content doc update: read body, look up title from folder metadata, upsert into search index.
fn search_handle_content_update(
    doc_id: &str,
    docs: &DashMap<String, DocWithSyncKv>,
    search_index: &SearchIndex,
) {
    let Some((_relay_id, doc_uuid)) = link_indexer::parse_doc_id(doc_id) else {
        return;
    };

    // Read Y.Text("contents") body
    let body = {
        let Some(doc_ref) = docs.get(doc_id) else {
            return;
        };
        let awareness = doc_ref.awareness();
        let guard = awareness.read().unwrap();
        let txn = guard.doc.transact();
        match txn.get_text("contents") {
            Some(text) => text.get_string(&txn),
            None => String::new(),
        }
    };

    // Find which folder doc contains this UUID and extract title
    let (title, folder_name) = search_find_title_and_folder(doc_uuid, docs);

    match search_index.add_document(doc_uuid, &title, &body, &folder_name) {
        Ok(()) => tracing::debug!("Search indexed content doc: {} ({})", doc_uuid, title),
        Err(e) => tracing::error!("Search index failed for {}: {:?}", doc_uuid, e),
    }
}

/// Find the title and folder name for a content doc UUID by scanning all folder docs' filemeta_v0.
fn search_find_title_and_folder(
    doc_uuid: &str,
    docs: &DashMap<String, DocWithSyncKv>,
) -> (String, String) {
    let folder_doc_ids = link_indexer::find_all_folder_docs(docs);

    for (folder_idx, folder_doc_id) in folder_doc_ids.iter().enumerate() {
        let Some(doc_ref) = docs.get(folder_doc_id) else {
            continue;
        };
        let awareness = doc_ref.awareness();
        let guard = awareness.read().unwrap();
        let txn = guard.doc.transact();
        let Some(filemeta) = txn.get_map("filemeta_v0") else {
            continue;
        };

        for (path, value) in filemeta.iter(&txn) {
            if let Some(id) = link_indexer::extract_id_from_filemeta_entry(&value, &txn) {
                if id == doc_uuid {
                    // Extract title: strip leading "/" and trailing ".md", take basename
                    let path_str: &str = path;
                    let title = path_str
                        .strip_prefix('/')
                        .unwrap_or(path_str)
                        .strip_suffix(".md")
                        .unwrap_or(path_str)
                        .rsplit('/')
                        .next()
                        .unwrap_or(path_str)
                        .to_string();

                    // Derive folder name from folder doc position
                    // First folder doc = "Lens", second = "Lens Edu"
                    let folder_name = if folder_idx == 0 {
                        "Lens".to_string()
                    } else {
                        "Lens Edu".to_string()
                    };

                    return (title, folder_name);
                }
            }
        }
    }

    // Not found in any folder doc — use UUID as title
    (doc_uuid.to_string(), "Unknown".to_string())
}

/// Handle folder doc update: detect added/removed UUIDs, update search index accordingly.
async fn search_handle_folder_update(
    folder_doc_id: &str,
    content_uuids: &[String],
    docs: &DashMap<String, DocWithSyncKv>,
    search_index: &SearchIndex,
    filemeta_cache: &DashMap<String, std::collections::HashMap<String, String>>,
    pending: &DashMap<String, tokio::time::Instant>,
    _rx: &tokio::sync::mpsc::Receiver<String>,
) {
    // Build current uuid -> title map from filemeta
    let current_map: std::collections::HashMap<String, String> = {
        let Some(doc_ref) = docs.get(folder_doc_id) else {
            return;
        };
        let awareness = doc_ref.awareness();
        let guard = awareness.read().unwrap();
        let txn = guard.doc.transact();
        let Some(filemeta) = txn.get_map("filemeta_v0") else {
            return;
        };

        let mut map = std::collections::HashMap::new();
        for (path, value) in filemeta.iter(&txn) {
            if let Some(id) = link_indexer::extract_id_from_filemeta_entry(&value, &txn) {
                let path_str: &str = path;
                let title = path_str
                    .strip_prefix('/')
                    .unwrap_or(path_str)
                    .strip_suffix(".md")
                    .unwrap_or(path_str)
                    .rsplit('/')
                    .next()
                    .unwrap_or(path_str)
                    .to_string();
                map.insert(id, title);
            }
        }
        map
    };

    // Get old snapshot from cache
    let old_map = filemeta_cache
        .get(folder_doc_id)
        .map(|r| r.clone());

    // Update cache with current snapshot
    filemeta_cache.insert(folder_doc_id.to_string(), current_map.clone());

    if let Some(old_map) = old_map {
        // Detect removed UUIDs
        for uuid in old_map.keys() {
            if !current_map.contains_key(uuid) {
                match search_index.remove_document(uuid) {
                    Ok(()) => tracing::info!("Search: removed doc {}", uuid),
                    Err(e) => tracing::error!("Search: failed to remove {}: {:?}", uuid, e),
                }
            }
        }

        // Detect added or renamed UUIDs — queue them for content indexing
        let Some((relay_id, _)) = link_indexer::parse_doc_id(folder_doc_id) else {
            return;
        };
        for (uuid, new_title) in &current_map {
            let old_title = old_map.get(uuid);
            if old_title.is_none() || old_title != Some(new_title) {
                // New or renamed — reindex content
                let content_id = format!("{}-{}", relay_id, uuid);
                if docs.contains_key(&content_id) {
                    pending.insert(content_id.clone(), tokio::time::Instant::now());
                    search_handle_content_update(&content_id, docs, search_index);
                }
            }
        }
    } else {
        // First time seeing this folder doc — index all content docs
        let Some((relay_id, _)) = link_indexer::parse_doc_id(folder_doc_id) else {
            return;
        };
        for uuid in content_uuids {
            let content_id = format!("{}-{}", relay_id, uuid);
            if docs.contains_key(&content_id) {
                search_handle_content_update(&content_id, docs, search_index);
            }
        }
    }
}

pub struct Server {
    docs: Arc<DashMap<String, DocWithSyncKv>>,
    doc_worker_tracker: TaskTracker,
    store: Option<Arc<Box<dyn Store>>>,
    checkpoint_freq: Duration,
    authenticator: Option<Authenticator>,
    url: Option<Url>,
    allowed_hosts: Vec<AllowedHost>,
    cancellation_token: CancellationToken,
    /// Whether to garbage collect docs that are no longer in use.
    /// Disabled for single-doc mode, since we only have one doc.
    /// Uses AtomicBool so it can be temporarily disabled during startup loading.
    doc_gc: std::sync::atomic::AtomicBool,
    event_dispatcher: Option<Arc<dyn EventDispatcher>>,
    sync_protocol_event_sender: Arc<SyncProtocolEventSender>,
    metrics: Arc<RelayMetrics>,
    link_indexer: Option<Arc<LinkIndexer>>,
    search_index: Option<Arc<SearchIndex>>,
    search_ready: Arc<std::sync::atomic::AtomicBool>,
    search_tx: Option<tokio::sync::mpsc::Sender<String>>,
    doc_resolver: Arc<DocumentResolver>,
    pub(crate) mcp_sessions: Arc<crate::mcp::session::SessionManager>,
    mcp_api_key: Option<String>,
}

impl Server {
    pub async fn new(
        store: Option<Box<dyn Store>>,
        checkpoint_freq: Duration,
        authenticator: Option<Authenticator>,
        url: Option<Url>,
        allowed_hosts: Vec<AllowedHost>,
        cancellation_token: CancellationToken,
        doc_gc: bool,
        webhook_configs: Option<Vec<WebhookConfig>>,
    ) -> Result<Self> {
        // Initialize metrics early so all senders can use them
        let metrics = RelayMetrics::new()
            .map_err(|e| anyhow!("Failed to initialize webhook metrics: {}", e))?;

        let sync_protocol_event_sender =
            Arc::new(SyncProtocolEventSender::new().with_metrics(metrics.clone()));

        let debounced_sync_sender = Arc::new(DebouncedSyncProtocolEventSender::new(
            sync_protocol_event_sender.clone(),
            metrics.clone(),
        ));

        let event_dispatcher = if let Some(configs) = webhook_configs {
            let webhook_sender = Arc::new(
                WebhookSender::new(configs.clone(), metrics.clone())
                    .map_err(|e| anyhow!("Failed to create webhook sender: {}", e))?,
            );

            let senders: Vec<Arc<dyn EventSender>> =
                vec![webhook_sender, debounced_sync_sender.clone()];

            Some(
                Arc::new(UnifiedEventDispatcher::new(senders, metrics.clone()))
                    as Arc<dyn EventDispatcher>,
            )
        } else {
            tracing::info!(
                "No webhook configs provided, creating sync protocol-only event dispatcher"
            );
            let senders: Vec<Arc<dyn EventSender>> = vec![debounced_sync_sender.clone()];
            Some(
                Arc::new(UnifiedEventDispatcher::new(senders, metrics.clone()))
                    as Arc<dyn EventDispatcher>,
            )
        };

        tracing::info!("Event dispatcher created successfully");

        let docs = Arc::new(DashMap::new());
        let (link_indexer, index_rx) = LinkIndexer::new();
        let link_indexer = Arc::new(link_indexer);

        // Spawn background worker for link indexing
        let docs_for_indexer = docs.clone();
        let indexer_for_worker = link_indexer.clone();
        tokio::spawn(async move {
            let result = std::panic::AssertUnwindSafe(
                indexer_for_worker.run_worker(index_rx, docs_for_indexer),
            );
            if let Err(e) = futures::FutureExt::catch_unwind(result).await {
                let msg = if let Some(s) = e.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = e.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "unknown panic payload".to_string()
                };
                tracing::error!("CRITICAL: Link indexer worker panicked: {msg}. Backlink indexing is now dead — restart the server.");
            } else {
                tracing::error!("CRITICAL: Link indexer worker exited unexpectedly (channel closed). Backlink indexing is now dead.");
            }
        });

        // Create SearchIndex with MmapDirectory in a temp directory
        let index_path = std::env::temp_dir().join("lens-relay-search-index");
        // Clean the directory on startup to ensure a fresh index
        if index_path.exists() {
            let _ = std::fs::remove_dir_all(&index_path);
        }
        let search_index = match SearchIndex::new(&index_path) {
            Ok(si) => {
                tracing::info!("SearchIndex created at {:?}", index_path);
                Some(Arc::new(si))
            }
            Err(e) => {
                tracing::error!("Failed to create SearchIndex: {:?}", e);
                None
            }
        };
        let search_ready = Arc::new(std::sync::atomic::AtomicBool::new(false));

        // Spawn background worker for search index updates
        let search_tx_final = if let Some(ref si) = search_index {
            let (search_tx, search_rx) = tokio::sync::mpsc::channel::<String>(1000);
            let si_for_worker = si.clone();
            let docs_for_search = docs.clone();
            let search_pending: Arc<DashMap<String, tokio::time::Instant>> =
                Arc::new(DashMap::new());

            tokio::spawn(async move {
                let result = std::panic::AssertUnwindSafe(search_worker(
                    search_rx,
                    si_for_worker,
                    docs_for_search,
                    search_pending,
                ));
                if let Err(e) = futures::FutureExt::catch_unwind(result).await {
                    let msg = if let Some(s) = e.downcast_ref::<&str>() {
                        s.to_string()
                    } else if let Some(s) = e.downcast_ref::<String>() {
                        s.clone()
                    } else {
                        "unknown panic payload".to_string()
                    };
                    tracing::error!("CRITICAL: Search index worker panicked: {msg}. Search indexing is now dead — restart the server.");
                } else {
                    tracing::error!("CRITICAL: Search index worker exited unexpectedly (channel closed). Search indexing is now dead.");
                }
            });

            Some(search_tx)
        } else {
            None
        };

        let mcp_api_key = std::env::var("MCP_API_KEY").ok();
        if mcp_api_key.is_some() {
            tracing::info!("MCP endpoint enabled (MCP_API_KEY is set)");
        } else {
            tracing::info!("MCP endpoint disabled (MCP_API_KEY not set)");
        }

        Ok(Self {
            docs,
            doc_worker_tracker: TaskTracker::new(),
            store: store.map(Arc::new),
            checkpoint_freq,
            authenticator,
            url,
            allowed_hosts,
            cancellation_token,
            doc_gc: std::sync::atomic::AtomicBool::new(doc_gc),
            event_dispatcher,
            sync_protocol_event_sender,
            metrics,
            link_indexer: Some(link_indexer),
            search_index,
            search_ready,
            search_tx: search_tx_final,
            doc_resolver: Arc::new(DocumentResolver::new()),
            mcp_sessions: Arc::new(crate::mcp::session::SessionManager::new()),
            mcp_api_key,
        })
    }

    /// Get the DocumentResolver for path-to-UUID resolution.
    pub fn doc_resolver(&self) -> &Arc<DocumentResolver> {
        &self.doc_resolver
    }

    /// Get the DashMap of all loaded documents.
    pub fn docs(&self) -> &Arc<DashMap<String, DocWithSyncKv>> {
        &self.docs
    }

    /// Create a minimal Server for testing. No store, no auth, no search.
    #[cfg(test)]
    pub fn new_for_test() -> Arc<Self> {
        Arc::new(Self {
            docs: Arc::new(DashMap::new()),
            doc_worker_tracker: TaskTracker::new(),
            store: None,
            checkpoint_freq: Duration::from_secs(60),
            authenticator: None,
            url: None,
            allowed_hosts: Vec::new(),
            cancellation_token: CancellationToken::new(),
            doc_gc: std::sync::atomic::AtomicBool::new(false),
            event_dispatcher: None,
            sync_protocol_event_sender: Arc::new(
                y_sweet_core::event::SyncProtocolEventSender::new(),
            ),
            metrics: RelayMetrics::new().expect("metrics init should not fail in tests"),
            link_indexer: None,
            search_index: None,
            search_ready: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            search_tx: None,
            doc_resolver: Arc::new(DocumentResolver::new()),
            mcp_sessions: Arc::new(crate::mcp::session::SessionManager::new()),
            mcp_api_key: None,
        })
    }

    pub async fn doc_exists(&self, doc_id: &str) -> bool {
        // Reject system keys
        if Self::validate_doc_id(doc_id).is_err() {
            return false;
        }
        if self.docs.contains_key(doc_id) {
            return true;
        }
        if let Some(store) = &self.store {
            store
                .exists(&format!("{}/data.ysweet", doc_id))
                .await
                .unwrap_or_default()
        } else {
            false
        }
    }

    pub async fn create_doc(&self) -> Result<String> {
        let doc_id = nanoid::nanoid!();
        self.load_doc(&doc_id, None).await?;
        tracing::info!(doc_id=?doc_id, "Created doc");
        Ok(doc_id)
    }

    pub async fn reload_webhook_config(&self) -> Result<String, anyhow::Error> {
        // For now, webhook configuration reloading is not supported with the new event system
        // This would require a more complex architecture to hot-reload the event dispatcher
        // In the meantime, server restart is required to change webhook configuration
        Err(anyhow::anyhow!(
            "Webhook configuration reloading is not yet supported with the new event system. Please restart the server to load new configuration."
        ))
    }

    fn validate_doc_id(doc_id: &str) -> Result<()> {
        // Reject system configuration paths that are reserved for internal use
        if doc_id.starts_with(".config/") || doc_id == ".config" {
            return Err(anyhow::anyhow!(
                "Document ID cannot access system configuration directory '.config'"
            ));
        }
        Ok(())
    }

    pub async fn load_doc(&self, doc_id: &str, routing_channel: Option<String>) -> Result<()> {
        self.load_doc_with_user(doc_id, routing_channel, None).await
    }

    pub async fn load_doc_with_user(
        &self,
        doc_id: &str,
        routing_channel: Option<String>,
        user: Option<String>,
    ) -> Result<()> {
        Self::validate_doc_id(doc_id)?;
        let (send, recv) = channel(1024);

        // Determine routing channel: use provided channel or fallback to doc_id
        let routing_channel_name = routing_channel
            .clone()
            .unwrap_or_else(|| doc_id.to_string());

        // Create event callback with the determined routing channel and user
        let event_callback = {
            let event_dispatcher = self.event_dispatcher.clone();
            let routing_channel_for_callback = routing_channel_name.clone();
            let user_for_callback = user.clone();
            let link_indexer_for_callback = self.link_indexer.clone();
            let search_tx_for_callback = self.search_tx.clone();
            let doc_key_for_indexer = doc_id.to_string();

            if let Some(dispatcher) = event_dispatcher {
                Some(Arc::new(move |mut event: DocumentUpdatedEvent, is_indexer: bool| {
                    // Add user to event if available
                    if let Some(ref user) = user_for_callback {
                        event.user = Some(user.clone());
                    }

                    // Log the full event payload as JSON after user assignment
                    match serde_json::to_string(&event) {
                        Ok(json_str) => {
                            tracing::info!("Document updated event dispatched: {}", json_str);
                        }
                        Err(e) => {
                            tracing::info!(
                                "Document updated event dispatched for doc_id: {} (JSON serialization failed: {})",
                                event.doc_id, e
                            );
                        }
                    }

                    // Step 1: Create the envelope with predetermined routing channel
                    let envelope = EventEnvelope::new(routing_channel_for_callback.clone(), event);

                    // Step 2: Send via dispatcher
                    dispatcher.send_event(envelope);

                    // Notify link indexer (if this update is not from the indexer itself)
                    if !is_indexer {
                        if let Some(ref indexer) = link_indexer_for_callback {
                            let indexer = indexer.clone();
                            let doc_key = doc_key_for_indexer.clone();
                            tokio::spawn(async move {
                                indexer.on_document_update(&doc_key).await;
                            });
                        }

                        // Notify search index worker
                        if let Some(ref tx) = search_tx_for_callback {
                            if let Err(e) = tx.try_send(doc_key_for_indexer.clone()) {
                                tracing::error!("Search index channel send failed (worker dead?): {e}");
                            }
                        }
                    }
                }) as y_sweet_core::webhook::WebhookCallback)
            } else {
                // Even without event dispatcher, we still want indexing
                let has_indexer = link_indexer_for_callback.is_some();
                let has_search = search_tx_for_callback.is_some();
                if has_indexer || has_search {
                    let indexer = link_indexer_for_callback.clone();
                    let search_tx = search_tx_for_callback.clone();
                    let doc_key = doc_key_for_indexer.clone();
                    Some(Arc::new(move |_event: DocumentUpdatedEvent, is_indexer: bool| {
                        if !is_indexer {
                            if let Some(ref indexer) = indexer {
                                let indexer = indexer.clone();
                                let doc_key = doc_key.clone();
                                tokio::spawn(async move {
                                    indexer.on_document_update(&doc_key).await;
                                });
                            }

                            // Notify search index worker
                            if let Some(ref tx) = search_tx {
                                if let Err(e) = tx.try_send(doc_key.clone()) {
                                    tracing::error!("Search index channel send failed (worker dead?): {e}");
                                }
                            }
                        }
                    }) as y_sweet_core::webhook::WebhookCallback)
                } else {
                    None
                }
            }
        };

        let dwskv = DocWithSyncKv::new(
            doc_id,
            self.store.clone(),
            move || {
                send.try_send(()).unwrap();
            },
            event_callback,
        )
        .await?;

        // If channel is provided in token, store it in document metadata
        if let Some(channel_name) = routing_channel {
            dwskv.set_channel(&channel_name);
        }

        dwskv
            .sync_kv()
            .persist()
            .await
            .map_err(|e| anyhow!("Error persisting: {:?}", e))?;

        {
            let sync_kv = dwskv.sync_kv();
            let checkpoint_freq = self.checkpoint_freq;
            let doc_id = doc_id.to_string();
            let cancellation_token = self.cancellation_token.clone();

            // Spawn a task to save the document to the store when it changes.
            self.doc_worker_tracker.spawn(
                Self::doc_persistence_worker(
                    recv,
                    sync_kv,
                    checkpoint_freq,
                    doc_id.clone(),
                    cancellation_token.clone(),
                )
                .instrument(span!(Level::INFO, "save_loop", doc_id=?doc_id)),
            );

            if self.doc_gc.load(std::sync::atomic::Ordering::Relaxed) {
                self.doc_worker_tracker.spawn(
                    Self::doc_gc_worker(
                        self.docs.clone(),
                        doc_id.clone(),
                        checkpoint_freq,
                        cancellation_token,
                    )
                    .instrument(span!(Level::INFO, "gc_loop", doc_id=?doc_id)),
                );
            }
        }

        self.docs.insert(doc_id.to_string(), dwskv);

        Ok(())
    }

    /// Load all documents from storage into memory.
    ///
    /// Enumerates all doc IDs in the store and calls `load_doc()` for each.
    /// Used on startup to populate the in-memory doc map before reindexing backlinks.
    pub async fn load_all_docs(&self) -> Result<usize> {
        let store = self.store.as_ref()
            .ok_or_else(|| anyhow!("No store configured — cannot load docs from storage"))?;

        let doc_ids = store.list_doc_ids().await
            .map_err(|e| anyhow!("Failed to list doc IDs from storage: {:?}", e))?;

        let total = doc_ids.len();
        tracing::info!("Loading {} documents from storage...", total);

        // Temporarily disable GC during bulk loading — all docs would be
        // immediately GCed since no clients are connected yet.
        let gc_was_enabled = self.doc_gc.swap(false, std::sync::atomic::Ordering::Relaxed);

        let mut loaded = 0;
        let mut failed = 0;

        for (i, doc_id) in doc_ids.iter().enumerate() {
            if self.docs.contains_key(doc_id) {
                loaded += 1;
                continue;
            }

            match self.load_doc(doc_id, None).await {
                Ok(()) => {
                    loaded += 1;
                    if (i + 1) % 50 == 0 || i + 1 == total {
                        tracing::info!("  Loaded {}/{} documents", i + 1, total);
                    }
                }
                Err(e) => {
                    tracing::warn!("  Failed to load doc {}: {:?}", doc_id, e);
                    failed += 1;
                }
            }
        }

        // Restore GC setting
        self.doc_gc.store(gc_was_enabled, std::sync::atomic::Ordering::Relaxed);

        tracing::info!(
            "Document loading complete: {} loaded, {} failed, {} total in storage",
            loaded, failed, total
        );
        Ok(loaded)
    }

    /// Load all documents from storage and reindex all backlinks.
    ///
    /// Called once on startup, before accepting connections.
    /// No-op if no store is configured (in-memory mode).
    pub async fn startup_reindex(&self) -> Result<()> {
        if self.store.is_none() {
            tracing::info!("No store configured, skipping startup reindex");
            // Even without a store, mark search as ready (empty index)
            self.search_ready
                .store(true, std::sync::atomic::Ordering::Release);
            return Ok(());
        }

        let loaded = self.load_all_docs().await?;
        tracing::info!("Loaded {} documents, now reindexing backlinks...", loaded);

        if let Some(ref indexer) = self.link_indexer {
            indexer.reindex_all_backlinks(&self.docs)?;
        }

        // Build document resolver (bidirectional path <-> UUID mapping)
        self.doc_resolver.rebuild(&self.docs);
        tracing::info!(
            "Document resolver built: {} documents",
            self.doc_resolver.all_paths().len()
        );

        // Build search index from all loaded documents
        if let Some(ref search_index) = self.search_index {
            tracing::info!("Building search index from loaded documents...");
            let mut indexed = 0;

            // Find all folder docs and build uuid -> (title, folder_name) map
            let folder_doc_ids = link_indexer::find_all_folder_docs(&self.docs);
            let mut uuid_metadata: std::collections::HashMap<String, (String, String)> =
                std::collections::HashMap::new();

            for (folder_idx, folder_doc_id) in folder_doc_ids.iter().enumerate() {
                let folder_name = if folder_idx == 0 {
                    "Lens".to_string()
                } else {
                    "Lens Edu".to_string()
                };

                let Some(doc_ref) = self.docs.get(folder_doc_id) else {
                    continue;
                };
                let awareness = doc_ref.awareness();
                let guard = awareness.read().unwrap();
                let txn = guard.doc.transact();
                let Some(filemeta) = txn.get_map("filemeta_v0") else {
                    continue;
                };

                for (path, value) in filemeta.iter(&txn) {
                    if let Some(uuid) =
                        link_indexer::extract_id_from_filemeta_entry(&value, &txn)
                    {
                        // Extract title: strip leading "/" and trailing ".md", take basename
                        let title = path
                            .strip_prefix('/')
                            .unwrap_or(&path)
                            .strip_suffix(".md")
                            .unwrap_or(&path)
                            .rsplit('/')
                            .next()
                            .unwrap_or(&path)
                            .to_string();
                        uuid_metadata.insert(uuid, (title, folder_name.clone()));
                    }
                }
            }

            tracing::info!(
                "Found {} documents in {} folder doc(s) for search indexing",
                uuid_metadata.len(),
                folder_doc_ids.len()
            );

            // For each UUID in the metadata map, find the content doc and index it
            for (uuid, (title, folder_name)) in &uuid_metadata {
                // Try to find the content doc — it might be under any relay_id prefix
                // Search through all loaded docs for one ending with this UUID
                let mut body = String::new();
                for entry in self.docs.iter() {
                    if let Some((_relay_id, doc_uuid)) =
                        link_indexer::parse_doc_id(entry.key())
                    {
                        if doc_uuid == uuid {
                            let awareness = entry.value().awareness();
                            let guard = awareness.read().unwrap();
                            let txn = guard.doc.transact();
                            if let Some(text) = txn.get_text("contents") {
                                body = text.get_string(&txn);
                            }
                            break;
                        }
                    }
                }

                match search_index.add_document(uuid, title, &body, folder_name) {
                    Ok(()) => indexed += 1,
                    Err(e) => {
                        tracing::error!(
                            "Failed to index doc {} into search: {:?}",
                            uuid,
                            e
                        );
                    }
                }
            }

            tracing::info!("Search index built: {} documents indexed", indexed);
        }

        // Mark search as ready after indexing is complete
        self.search_ready
            .store(true, std::sync::atomic::Ordering::Release);
        tracing::info!("Search index is now ready for queries");

        Ok(())
    }

    async fn doc_gc_worker(
        docs: Arc<DashMap<String, DocWithSyncKv>>,
        doc_id: String,
        checkpoint_freq: Duration,
        cancellation_token: CancellationToken,
    ) {
        let mut checkpoints_without_refs = 0;

        loop {
            tokio::select! {
                _ = tokio::time::sleep(checkpoint_freq) => {
                    if let Some(doc) = docs.get(&doc_id) {
                        let awareness = Arc::downgrade(&doc.awareness());
                        if awareness.strong_count() > 1 {
                            checkpoints_without_refs = 0;
                            tracing::debug!("doc is still alive - it has {} references", awareness.strong_count());
                        } else {
                            checkpoints_without_refs += 1;
                            tracing::info!("doc has only one reference, candidate for GC. checkpoints_without_refs: {}", checkpoints_without_refs);
                        }
                    } else {
                        break;
                    }

                    if checkpoints_without_refs >= 2 {
                        tracing::info!("GCing doc");
                        docs.remove(&doc_id);
                        break;
                    }
                }
                _ = cancellation_token.cancelled() => {
                    break;
                }
            };
        }
        tracing::info!("Exiting gc_loop");
    }

    async fn doc_persistence_worker(
        mut recv: Receiver<()>,
        sync_kv: Arc<SyncKv>,
        checkpoint_freq: Duration,
        doc_id: String,
        cancellation_token: CancellationToken,
    ) {
        let mut last_save = std::time::Instant::now();

        loop {
            let is_done = tokio::select! {
                v = recv.recv() => v.is_none(),
                _ = cancellation_token.cancelled() => true,
            };

            tracing::info!("Received signal. done: {}", is_done);
            let now = std::time::Instant::now();
            if !is_done && now - last_save < checkpoint_freq {
                let sleep = tokio::time::sleep(checkpoint_freq - (now - last_save));
                tokio::pin!(sleep);
                tracing::info!("Throttling.");

                loop {
                    tokio::select! {
                        _ = &mut sleep => {
                            break;
                        }
                        v = recv.recv() => {
                            tracing::info!("Received dirty while throttling.");
                            if v.is_none() {
                                break;
                            }
                        }
                        _ = cancellation_token.cancelled() => {
                            tracing::info!("Received cancellation while throttling.");
                            break;
                        }

                    }
                    tracing::info!("Done throttling.");
                }
            }
            tracing::info!("Persisting.");
            if let Err(e) = sync_kv.persist().await {
                tracing::error!(?e, "Error persisting.");
            } else {
                tracing::info!("Done persisting.");
            }
            last_save = std::time::Instant::now();

            if is_done {
                break;
            }
        }
        tracing::info!("Terminating loop for {}", doc_id);
    }

    pub async fn get_or_create_doc(
        &self,
        doc_id: &str,
    ) -> Result<MappedRef<String, DocWithSyncKv, DocWithSyncKv>> {
        if !self.docs.contains_key(doc_id) {
            tracing::info!(doc_id=?doc_id, "Loading doc");
            self.load_doc(doc_id, None).await?;
        }

        Ok(self
            .docs
            .get(doc_id)
            .ok_or_else(|| anyhow!("Failed to get-or-create doc"))?
            .map(|d| d))
    }

    pub async fn get_or_create_doc_with_channel(
        &self,
        doc_id: &str,
        routing_channel: Option<String>,
    ) -> Result<MappedRef<String, DocWithSyncKv, DocWithSyncKv>> {
        self.get_or_create_doc_with_channel_and_user(doc_id, routing_channel, None)
            .await
    }

    pub async fn get_or_create_doc_with_channel_and_user(
        &self,
        doc_id: &str,
        routing_channel: Option<String>,
        user: Option<String>,
    ) -> Result<MappedRef<String, DocWithSyncKv, DocWithSyncKv>> {
        if !self.docs.contains_key(doc_id) {
            tracing::info!(doc_id=?doc_id, channel=?routing_channel, user=?user, "Loading doc with channel and user");
            self.load_doc_with_user(doc_id, routing_channel, user)
                .await?;
        }

        Ok(self
            .docs
            .get(doc_id)
            .ok_or_else(|| anyhow!("Failed to get-or-create doc"))?
            .map(|d| d))
    }

    pub fn check_auth(
        &self,
        auth_header: Option<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>,
    ) -> Result<(), AppError> {
        if let Some(auth) = &self.authenticator {
            if let Some(TypedHeader(headers::Authorization(bearer))) = auth_header {
                if let Ok(()) =
                    auth.verify_server_token(bearer.token(), current_time_epoch_millis())
                {
                    return Ok(());
                }
            }
            Err((StatusCode::UNAUTHORIZED, anyhow!("Unauthorized.")))?
        } else {
            Ok(())
        }
    }

    pub async fn redact_error_middleware(req: Request, next: Next) -> impl IntoResponse {
        let resp = next.run(req).await;
        if resp.status().is_server_error() || resp.status().is_client_error() {
            // If we should redact errors, copy over only the status code and
            // not the response body.
            return resp.status().into_response();
        }
        resp
    }

    pub async fn version_header_middleware(req: Request, next: Next) -> impl IntoResponse {
        let mut resp = next.run(req).await;
        resp.headers_mut().insert(
            HeaderName::from_static("relay-server-version"),
            HeaderValue::from_static(RELAY_SERVER_VERSION),
        );
        resp
    }

    pub fn routes(self: &Arc<Self>) -> Router {
        let mut router = Router::new()
            .route("/ready", get(ready))
            .route("/check_store", post(check_store))
            .route("/check_store", get(check_store_deprecated))
            .route("/doc/ws/:doc_id", get(handle_socket_upgrade_deprecated))
            .route("/doc/new", post(new_doc))
            .route("/doc/:doc_id/auth", post(auth_doc))
            .route("/doc/:doc_id/as-update", get(get_doc_as_update_deprecated))
            .route("/doc/:doc_id/update", post(update_doc_deprecated))
            .route("/d/:doc_id/as-update", get(get_doc_as_update))
            .route("/d/:doc_id/update", post(update_doc))
            .route("/d/:doc_id/versions", get(handle_doc_versions))
            .route(
                "/d/:doc_id/ws/:doc_id2",
                get(handle_socket_upgrade_full_path),
            )
            .route("/webhook/reload", post(reload_webhook_config_endpoint))
            .route("/search", get(handle_search));

        // Only register /mcp if MCP_API_KEY is set
        if let Some(ref key) = self.mcp_api_key {
            let mcp_routes = Router::new()
                .route(
                    "/",
                    post(crate::mcp::transport::handle_mcp_post)
                        .get(crate::mcp::transport::handle_mcp_get)
                        .delete(crate::mcp::transport::handle_mcp_delete),
                )
                .layer(middleware::from_fn_with_state(
                    key.clone(),
                    crate::mcp::transport::mcp_auth_middleware,
                ))
                .with_state(self.clone());
            router = router.nest("/mcp", mcp_routes);
        }

        // Only add file endpoints if a store is configured
        if let Some(store) = &self.store {
            // Add presigned URL endpoints for all stores
            router = router
                .route("/f/:doc_id/upload-url", post(handle_file_upload_url))
                .route("/f/:doc_id/download-url", get(handle_file_download_url));

            // Add file operations that work with any store
            router = router
                .route("/f/:doc_id/history", get(handle_file_history))
                .route("/f/:doc_id", delete(handle_file_delete))
                .route("/f/:doc_id/:hash", delete(handle_file_delete_by_hash))
                .route("/f/:doc_id", head(handle_file_head));

            // Only add direct upload/download endpoints if store supports direct uploads
            if store.supports_direct_uploads() {
                router = router
                    .route(
                        "/f/:doc_id/upload",
                        post(handle_file_upload).put(handle_file_upload_raw),
                    )
                    .route("/f/:doc_id/download", get(handle_file_download));
            }
        }

        router.with_state(self.clone())
    }

    pub fn single_doc_routes(self: &Arc<Self>) -> Router {
        Router::new()
            .route("/ws/:doc_id", get(handle_socket_upgrade_single))
            .route("/as-update", get(get_doc_as_update_single))
            .route("/update", post(update_doc_single))
            .with_state(self.clone())
    }

    pub fn metrics_routes(self: &Arc<Self>) -> Router {
        Router::new()
            .route("/metrics", get(metrics_endpoint))
            .with_state(self.clone())
    }

    async fn serve_internal(
        self: Arc<Self>,
        listener: TcpListener,
        redact_errors: bool,
        routes: Router,
    ) -> Result<()> {
        let token = self.cancellation_token.clone();

        let app = routes.layer(middleware::from_fn(Self::version_header_middleware));
        let app = if redact_errors {
            app
        } else {
            app.layer(middleware::from_fn(Self::redact_error_middleware))
        };

        tracing::info!("Starting HTTP server...");
        axum::serve(listener, app.into_make_service())
            .with_graceful_shutdown(async move {
                tracing::info!("Waiting for cancellation token...");
                token.cancelled().await;
                tracing::info!("Cancellation token triggered, starting graceful shutdown");
            })
            .await?;

        tracing::info!("HTTP server stopped, shutting down event dispatcher...");

        // Explicitly shutdown event dispatcher before waiting on doc workers
        if let Some(event_dispatcher) = &self.event_dispatcher {
            tracing::info!("Shutting down event dispatcher...");
            event_dispatcher.shutdown();
            tracing::info!("Event dispatcher shutdown complete");
        }

        tracing::info!("Closing doc worker tracker...");
        self.doc_worker_tracker.close();
        tracing::info!("Waiting for doc workers to finish...");
        self.doc_worker_tracker.wait().await;
        tracing::info!("All doc workers stopped");

        Ok(())
    }

    pub async fn serve(self, listener: TcpListener, redact_errors: bool) -> Result<()> {
        let s = Arc::new(self);
        let routes = s.routes();
        s.serve_internal(listener, redact_errors, routes).await
    }

    pub async fn serve_doc(self, listener: TcpListener, redact_errors: bool) -> Result<()> {
        let s = Arc::new(self);
        let routes = s.single_doc_routes();
        s.serve_internal(listener, redact_errors, routes).await
    }

    pub async fn serve_metrics(self, listener: TcpListener) -> Result<()> {
        let s = Arc::new(self);
        let routes = s.metrics_routes();
        s.serve_internal(listener, false, routes).await
    }

    fn verify_doc_token(&self, token: Option<&str>, doc: &str) -> Result<Authorization, AppError> {
        if let Some(authenticator) = &self.authenticator {
            if let Some(token) = token {
                let authorization = authenticator
                    .verify_doc_token(token, doc, current_time_epoch_millis())
                    .map_err(|e| (StatusCode::UNAUTHORIZED, e))?;
                Ok(authorization)
            } else {
                Err((StatusCode::UNAUTHORIZED, anyhow!("No token provided.")))?
            }
        } else {
            Ok(Authorization::Full)
        }
    }

    fn get_single_doc_id(&self) -> Result<String, AppError> {
        self.docs
            .iter()
            .next()
            .map(|entry| entry.key().clone())
            .ok_or_else(|| AppError(StatusCode::NOT_FOUND, anyhow!("No document found")))
    }
}

#[derive(Deserialize)]
struct HandlerParams {
    token: Option<String>,
}

async fn get_doc_as_update(
    State(server_state): State<Arc<Server>>,
    Path(doc_id): Path<String>,
    auth_header: Option<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>,
) -> Result<Response, AppError> {
    // All authorization types allow reading the document.
    let token = get_token_from_header(auth_header);
    let _ = server_state.verify_doc_token(token.as_deref(), &doc_id)?;

    let dwskv = server_state
        .get_or_create_doc(&doc_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let update = dwskv.as_update();
    tracing::debug!("update: {:?}", update);
    Ok(update.into_response())
}

async fn get_doc_as_update_deprecated(
    Path(doc_id): Path<String>,
    State(server_state): State<Arc<Server>>,
    auth_header: Option<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>,
) -> Result<Response, AppError> {
    tracing::warn!("/doc/:doc_id/as-update is deprecated; call /doc/:doc_id/auth instead and then call as-update on the returned base URL.");
    get_doc_as_update(State(server_state), Path(doc_id), auth_header).await
}

async fn update_doc_deprecated(
    Path(doc_id): Path<String>,
    State(server_state): State<Arc<Server>>,
    auth_header: Option<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>,
    body: Bytes,
) -> Result<Response, AppError> {
    tracing::warn!("/doc/:doc_id/update is deprecated; call /doc/:doc_id/auth instead and then call update on the returned base URL.");
    update_doc(Path(doc_id), State(server_state), auth_header, body).await
}

async fn get_doc_as_update_single(
    State(server_state): State<Arc<Server>>,
    auth_header: Option<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>,
) -> Result<Response, AppError> {
    let doc_id = server_state.get_single_doc_id()?;
    get_doc_as_update(State(server_state), Path(doc_id), auth_header).await
}

async fn update_doc(
    Path(doc_id): Path<String>,
    State(server_state): State<Arc<Server>>,
    auth_header: Option<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>,
    body: Bytes,
) -> Result<Response, AppError> {
    let token = get_token_from_header(auth_header);
    let authorization = server_state.verify_doc_token(token.as_deref(), &doc_id)?;
    update_doc_inner(doc_id, server_state, authorization, body).await
}

async fn update_doc_inner(
    doc_id: String,
    server_state: Arc<Server>,
    authorization: Authorization,
    body: Bytes,
) -> Result<Response, AppError> {
    if !matches!(authorization, Authorization::Full) {
        return Err(AppError(StatusCode::FORBIDDEN, anyhow!("Unauthorized.")));
    }

    let dwskv = server_state
        .get_or_create_doc(&doc_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    if let Err(err) = dwskv.apply_update(&body) {
        tracing::error!(?err, "Failed to apply update");
        return Err(AppError(StatusCode::INTERNAL_SERVER_ERROR, err));
    }

    Ok(StatusCode::OK.into_response())
}

async fn update_doc_single(
    State(server_state): State<Arc<Server>>,
    auth_header: Option<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>,
    body: Bytes,
) -> Result<Response, AppError> {
    let doc_id = server_state.get_single_doc_id()?;
    let token = get_token_from_header(auth_header);
    let authorization = server_state.verify_doc_token(token.as_deref(), &doc_id)?;
    update_doc_inner(doc_id, server_state, authorization, body).await
}

async fn handle_socket_upgrade(
    ws: WebSocketUpgrade,
    Path(doc_id): Path<String>,
    authorization: Authorization,
    State(server_state): State<Arc<Server>>,
) -> Result<Response, AppError> {
    handle_socket_upgrade_with_channel(ws, Path(doc_id), authorization, None, State(server_state))
        .await
}

async fn handle_socket_upgrade_with_channel(
    ws: WebSocketUpgrade,
    Path(doc_id): Path<String>,
    authorization: Authorization,
    routing_channel: Option<String>,
    State(server_state): State<Arc<Server>>,
) -> Result<Response, AppError> {
    handle_socket_upgrade_with_channel_and_user(
        ws,
        Path(doc_id),
        authorization,
        routing_channel,
        None,
        None, // No token available at this level
        State(server_state),
    )
    .await
}

async fn handle_socket_upgrade_with_channel_and_user(
    ws: WebSocketUpgrade,
    Path(doc_id): Path<String>,
    authorization: Authorization,
    routing_channel: Option<String>,
    user: Option<String>,
    token: Option<String>,
    State(server_state): State<Arc<Server>>,
) -> Result<Response, AppError> {
    if !matches!(authorization, Authorization::Full) && !server_state.docs.contains_key(&doc_id) {
        return Err(AppError(
            StatusCode::NOT_FOUND,
            anyhow!("Doc {} not found", doc_id),
        ));
    }

    // Extract expiration time from token
    let expiration_time = if let Some(authenticator) = &server_state.authenticator {
        if let Some(token_str) = token.as_deref() {
            authenticator
                .decode_token(token_str)
                .ok()
                .and_then(|payload| payload.expiration_millis)
                .map(|exp| exp.0)
        } else {
            None
        }
    } else {
        None
    };

    let dwskv = server_state
        .get_or_create_doc_with_channel_and_user(&doc_id, routing_channel, user)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let awareness = dwskv.awareness();
    let cancellation_token = server_state.cancellation_token.clone();
    let sync_protocol_event_sender = server_state.sync_protocol_event_sender.clone();
    let metrics = server_state.metrics.clone();
    let doc_id_clone = doc_id.clone();

    Ok(ws.on_upgrade(move |socket| {
        handle_socket(
            socket,
            awareness,
            authorization,
            expiration_time,
            cancellation_token,
            sync_protocol_event_sender,
            doc_id_clone,
            metrics,
        )
    }))
}

async fn handle_socket_upgrade_deprecated(
    ws: WebSocketUpgrade,
    Path(doc_id): Path<String>,
    Query(params): Query<HandlerParams>,
    State(server_state): State<Arc<Server>>,
) -> Result<Response, AppError> {
    tracing::warn!(
        "/doc/ws/:doc_id is deprecated; call /doc/:doc_id/auth instead and use the returned URL."
    );
    let (permission, channel) = if let Some(authenticator) = &server_state.authenticator {
        if let Some(token) = params.token.as_deref() {
            authenticator
                .verify_token_with_channel(token, current_time_epoch_millis())
                .map_err(|e| {
                    // Record authentication failure metric
                    server_state.metrics.record_auth_failure(
                        e.to_metric_label(),
                        "websocket_upgrade_deprecated",
                        "GET",
                    );
                    (StatusCode::UNAUTHORIZED, e)
                })?
        } else {
            // Record missing token when authenticator is present but no token provided
            server_state
                .metrics
                .record_missing_token("websocket_upgrade_deprecated", "true");
            (y_sweet_core::auth::Permission::Server, None)
        }
    } else {
        (y_sweet_core::auth::Permission::Server, None)
    };

    let (authorization, user) = match permission {
        y_sweet_core::auth::Permission::Doc(doc_perm) => {
            if doc_perm.doc_id != doc_id {
                return Err(AppError(
                    StatusCode::FORBIDDEN,
                    anyhow!("Token not valid for this document"),
                ));
            }
            (doc_perm.authorization, doc_perm.user)
        }
        y_sweet_core::auth::Permission::Server => (Authorization::Full, None),
        y_sweet_core::auth::Permission::Prefix(prefix_perm) => {
            if !doc_id.starts_with(&prefix_perm.prefix) {
                return Err(AppError(
                    StatusCode::FORBIDDEN,
                    anyhow!("Token not valid for this document"),
                ));
            }
            (prefix_perm.authorization, prefix_perm.user)
        }
        y_sweet_core::auth::Permission::File(_) => {
            return Err(AppError(
                StatusCode::FORBIDDEN,
                anyhow!("File token not valid for document access"),
            ));
        }
    };

    handle_socket_upgrade_with_channel_and_user(
        ws,
        Path(doc_id),
        authorization,
        channel,
        user,
        params.token.clone(), // Pass the token from query params
        State(server_state),
    )
    .await
}

async fn handle_socket_upgrade_full_path(
    ws: WebSocketUpgrade,
    Path((doc_id, doc_id2)): Path<(String, String)>,
    Query(params): Query<HandlerParams>,
    State(server_state): State<Arc<Server>>,
) -> Result<Response, AppError> {
    tracing::debug!("WebSocket upgrade request for doc: {}", doc_id);

    if doc_id != doc_id2 {
        tracing::debug!("Doc ID mismatch: {} != {}", doc_id, doc_id2);
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            anyhow!("For Yjs compatibility, the doc_id appears twice in the URL. It must be the same in both places, but we got {} and {}.", doc_id, doc_id2),
        ));
    }

    let (permission, channel) = if let Some(authenticator) = &server_state.authenticator {
        if let Some(token) = params.token.as_deref() {
            let current_time = current_time_epoch_millis();

            authenticator
                .verify_token_with_channel(token, current_time)
                .map_err(|e| {
                    tracing::debug!("Token verification failed: {:?}", e);
                    (StatusCode::UNAUTHORIZED, e)
                })?
        } else {
            (y_sweet_core::auth::Permission::Server, None)
        }
    } else {
        (y_sweet_core::auth::Permission::Server, None)
    };

    let (authorization, user) = match permission {
        y_sweet_core::auth::Permission::Doc(doc_perm) => {
            if doc_perm.doc_id != doc_id {
                return Err(AppError(
                    StatusCode::FORBIDDEN,
                    anyhow!("Token not valid for this document"),
                ));
            }
            (doc_perm.authorization, doc_perm.user)
        }
        y_sweet_core::auth::Permission::Server => (Authorization::Full, None),
        y_sweet_core::auth::Permission::Prefix(prefix_perm) => {
            if !doc_id.starts_with(&prefix_perm.prefix) {
                return Err(AppError(
                    StatusCode::FORBIDDEN,
                    anyhow!("Token not valid for this document"),
                ));
            }
            (prefix_perm.authorization, prefix_perm.user)
        }
        y_sweet_core::auth::Permission::File(_) => {
            return Err(AppError(
                StatusCode::FORBIDDEN,
                anyhow!("File token not valid for document access"),
            ));
        }
    };

    handle_socket_upgrade_with_channel_and_user(
        ws,
        Path(doc_id),
        authorization,
        channel,
        user,
        params.token.clone(), // Pass the token from query params
        State(server_state),
    )
    .await
}

async fn handle_socket_upgrade_single(
    ws: WebSocketUpgrade,
    Path(doc_id): Path<String>,
    auth_header: Option<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>,
    State(server_state): State<Arc<Server>>,
) -> Result<Response, AppError> {
    let single_doc_id = server_state.get_single_doc_id()?;
    if doc_id != single_doc_id {
        return Err(AppError(
            StatusCode::NOT_FOUND,
            anyhow!("Document not found"),
        ));
    }

    let token = get_token_from_header(auth_header);
    let authorization = server_state.verify_doc_token(token.as_deref(), &doc_id)?;
    handle_socket_upgrade(ws, Path(single_doc_id), authorization, State(server_state)).await
}

async fn handle_socket(
    socket: WebSocket,
    awareness: Arc<RwLock<Awareness>>,
    authorization: Authorization,
    expiration_time: Option<u64>,
    cancellation_token: CancellationToken,
    sync_protocol_event_sender: Arc<SyncProtocolEventSender>,
    doc_id: String,
    metrics: Arc<RelayMetrics>,
) {
    let (mut sink, mut stream) = socket.split();
    let (send, mut recv) = channel(1024);

    tokio::spawn(async move {
        while let Some(msg) = recv.recv().await {
            let _ = sink.send(msg).await;
        }
    });

    let send_clone = send.clone();
    let connection = Arc::new(DocConnection::new_with_expiration(
        awareness,
        authorization,
        expiration_time,
        move |bytes| {
            if let Err(e) = send_clone.try_send(Message::Binary(bytes.to_vec())) {
                tracing::warn!(?e, "Error sending message");
            }
        },
    ));

    // Register the connection with the sync protocol event sender
    sync_protocol_event_sender.register_doc_connection(doc_id.clone(), Arc::downgrade(&connection));

    loop {
        tokio::select! {
            Some(msg) = stream.next() => {
                let msg = match msg {
                    Ok(Message::Binary(bytes)) => bytes,
                    Ok(Message::Close(_)) => break,
                    Err(_e) => {
                        // The stream will complain about things like
                        // connections being lost without handshake.
                        continue;
                    }
                    msg => {
                        tracing::warn!(?msg, "Received non-binary message");
                        continue;
                    }
                };

                match connection.send(&msg).await {
                    Ok(_) => {},
                    Err(e) if e.to_string().contains("Token expired") => {
                        // Record token expiration metric
                        metrics.record_token_expired(
                            "websocket_connection",
                            "websocket_message_handler"
                        );
                        tracing::warn!(
                            doc_id = %doc_id,
                            "Closing connection due to token expiration"
                        );
                        let _ = send.try_send(Message::Close(Some(CloseFrame {
                            code: 1008, // Policy Violation - indicates a policy violation
                            reason: "Token expired".into(),
                        })));
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(?e, "Error handling message");
                    }
                }
            }
            _ = cancellation_token.cancelled() => {
                tracing::debug!("Closing doc connection due to server cancel...");
                break;
            }
        }
    }
}

async fn check_store(
    auth_header: Option<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>,
    State(server_state): State<Arc<Server>>,
) -> Result<Json<Value>, AppError> {
    server_state.check_auth(auth_header)?;

    if server_state.store.is_none() {
        return Ok(Json(json!({"ok": false, "error": "No store set."})));
    };

    // The check_store endpoint for the native server is kind of moot, since
    // the server will not start if store is not ok.
    Ok(Json(json!({"ok": true})))
}

async fn check_store_deprecated(
    auth_header: Option<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>,
    State(server_state): State<Arc<Server>>,
) -> Result<Json<Value>, AppError> {
    tracing::warn!(
        "GET check_store is deprecated, use POST check_store with an empty body instead."
    );
    check_store(auth_header, State(server_state)).await
}

/// Always returns a 200 OK response, as long as we are listening.
async fn ready() -> Result<Json<Value>, AppError> {
    Ok(Json(json!({"ok": true})))
}

async fn handle_search(
    State(server_state): State<Arc<Server>>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<Value>, AppError> {
    // Check if search is ready (503 during initial indexing)
    if !server_state
        .search_ready
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(AppError(
            StatusCode::SERVICE_UNAVAILABLE,
            anyhow!("Search index is being built, please try again shortly"),
        ));
    }

    let limit = params.limit.min(100); // Cap at 100
    let q = params.q.trim().to_string();

    if q.is_empty() {
        return Ok(Json(json!({
            "results": [],
            "total_hits": 0,
            "query": ""
        })));
    }

    let search_index = server_state.search_index.clone().ok_or_else(|| {
        AppError(
            StatusCode::SERVICE_UNAVAILABLE,
            anyhow!("Search index not available"),
        )
    })?;

    // Run search in blocking context (tantivy is sync)
    let results = tokio::task::spawn_blocking(move || search_index.search(&q, limit))
        .await
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.into()))?
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let total_hits = results.len();
    Ok(Json(json!({
        "results": results,
        "total_hits": total_hits,
        "query": params.q
    })))
}

async fn new_doc(
    auth_header: Option<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>,
    State(server_state): State<Arc<Server>>,
    Json(body): Json<DocCreationRequest>,
) -> Result<Json<NewDocResponse>, AppError> {
    let token = get_token_from_header(auth_header);

    if let Some(authenticator) = &server_state.authenticator {
        if let Some(token) = token.as_deref() {
            // First try server token
            if authenticator
                .verify_server_token(token, current_time_epoch_millis())
                .is_ok()
            {
                // Server token allows creating any document
            } else {
                // Try prefix token - we need to check if the doc_id matches the prefix
                if let Some(doc_id) = &body.doc_id {
                    let permission = authenticator
                        .verify_token_auto(token, current_time_epoch_millis())
                        .map_err(|auth_error| {
                            server_state.metrics.record_auth_failure(
                                auth_error.to_metric_label(),
                                "new_doc",
                                "POST",
                            );
                            AppError(
                                StatusCode::UNAUTHORIZED,
                                anyhow!("Invalid token: {}", auth_error),
                            )
                        })?;

                    match permission {
                        Permission::Prefix(prefix_perm) => {
                            // Check if the document ID starts with the prefix
                            if !doc_id.starts_with(&prefix_perm.prefix) {
                                server_state.metrics.record_permission_denied(
                                    "document",
                                    "prefix_mismatch",
                                    "new_doc",
                                );
                                return Err(AppError(
                                    StatusCode::FORBIDDEN,
                                    anyhow!(
                                        "Document ID '{}' does not match prefix '{}'",
                                        doc_id,
                                        prefix_perm.prefix
                                    ),
                                ));
                            }
                            // Check if we have Full permissions (needed for creation)
                            if prefix_perm.authorization != Authorization::Full {
                                server_state.metrics.record_permission_denied(
                                    "document",
                                    "insufficient_permissions",
                                    "new_doc",
                                );
                                return Err(AppError(
                                    StatusCode::FORBIDDEN,
                                    anyhow!("Prefix token requires Full authorization to create documents")
                                ));
                            }
                        }
                        _ => {
                            server_state.metrics.record_permission_denied(
                                "document",
                                "wrong_token_type",
                                "new_doc",
                            );
                            return Err(AppError(
                                StatusCode::FORBIDDEN,
                                anyhow!("Only server or prefix tokens can create documents"),
                            ));
                        }
                    }
                } else {
                    // No doc_id provided - only server tokens can create with auto-generated ID
                    return Err(AppError(
                        StatusCode::FORBIDDEN,
                        anyhow!("Prefix tokens must specify a docId that matches their prefix"),
                    ));
                }
            }
        } else {
            server_state.metrics.record_missing_token("new_doc", "true");
            return Err(AppError(
                StatusCode::UNAUTHORIZED,
                anyhow!("No token provided"),
            ));
        }
    }

    let doc_id = if let Some(doc_id) = body.doc_id {
        if !validate_doc_name(doc_id.as_str()) {
            Err((StatusCode::BAD_REQUEST, anyhow!("Invalid document name")))?
        }

        server_state
            .get_or_create_doc(doc_id.as_str())
            .await
            .map_err(|e| {
                tracing::error!(?e, "Failed to create doc");
                (StatusCode::INTERNAL_SERVER_ERROR, e)
            })?;

        doc_id
    } else {
        server_state.create_doc().await.map_err(|d| {
            tracing::error!(?d, "Failed to create doc");
            (StatusCode::INTERNAL_SERVER_ERROR, d)
        })?
    };

    Ok(Json(NewDocResponse { doc_id }))
}

fn generate_base_url(
    url: &Option<Url>,
    allowed_hosts: &[AllowedHost],
    request_host: &str,
) -> Result<String, AppError> {
    // Priority 1: Explicit URL prefix
    if let Some(prefix) = url {
        return Ok(prefix.as_str().trim_end_matches('/').to_string());
    }

    // Priority 2: Context-derived URL from Host header
    if let Some(allowed) = allowed_hosts.iter().find(|h| h.host == request_host) {
        return Ok(format!("{}://{}", allowed.scheme, request_host));
    }

    // Priority 3: Fallback to old behavior for backward compatibility
    if allowed_hosts.is_empty() {
        return Ok(format!("http://{}", request_host));
    }

    // Reject unknown hosts when allowed_hosts is configured
    Err(AppError(
        StatusCode::BAD_REQUEST,
        anyhow!("Host '{}' not in allowed hosts list", request_host),
    ))
}

fn generate_context_aware_urls(
    url: &Option<Url>,
    allowed_hosts: &[AllowedHost],
    request_host: &str,
    doc_id: &str,
) -> Result<(String, String), AppError> {
    // Priority 1: Explicit URL prefix
    if let Some(prefix) = url {
        let ws_scheme = if prefix.scheme() == "https" {
            "wss"
        } else {
            "ws"
        };
        let mut ws_url = prefix.clone();
        ws_url.set_scheme(ws_scheme).unwrap();
        let ws_url = ws_url
            .join(&format!("/d/{}/ws", doc_id))
            .unwrap()
            .to_string();

        let base_url = format!("{}/d/{}", prefix.as_str().trim_end_matches('/'), doc_id);
        return Ok((ws_url, base_url));
    }

    // Priority 2: Context-derived URL from Host header
    if let Some(allowed) = allowed_hosts.iter().find(|h| h.host == request_host) {
        let ws_scheme = if allowed.scheme == "https" {
            "wss"
        } else {
            "ws"
        };
        let ws_url = format!("{}://{}/d/{}/ws", ws_scheme, request_host, doc_id);
        let base_url = format!("{}://{}/d/{}", allowed.scheme, request_host, doc_id);
        return Ok((ws_url, base_url));
    }

    // Priority 3: Fallback to old behavior for backward compatibility
    // This handles the case where no URL prefix and no allowed hosts are set
    if allowed_hosts.is_empty() {
        let ws_url = format!("ws://{}/d/{}/ws", request_host, doc_id);
        let base_url = format!("http://{}/d/{}", request_host, doc_id);
        return Ok((ws_url, base_url));
    }

    // Reject unknown hosts when allowed_hosts is configured
    Err(AppError(
        StatusCode::BAD_REQUEST,
        anyhow!("Host '{}' not in allowed hosts list", request_host),
    ))
}

async fn auth_doc(
    auth_header: Option<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>,
    TypedHeader(host): TypedHeader<headers::Host>,
    State(server_state): State<Arc<Server>>,
    Path(doc_id): Path<String>,
    body: Option<Json<AuthDocRequest>>,
) -> Result<Json<ClientToken>, AppError> {
    server_state.check_auth(auth_header)?;

    let Json(AuthDocRequest {
        authorization,
        valid_for_seconds,
        ..
    }) = body.unwrap_or_default();

    if !server_state.doc_exists(&doc_id).await {
        Err((StatusCode::NOT_FOUND, anyhow!("Doc {} not found", doc_id)))?;
    }

    let valid_for_seconds = valid_for_seconds.unwrap_or(DEFAULT_EXPIRATION_SECONDS);
    let expiration_time =
        ExpirationTimeEpochMillis(current_time_epoch_millis() + valid_for_seconds * 1000);

    let token = if let Some(auth) = &server_state.authenticator {
        let token = auth
            .gen_doc_token_auto(&doc_id, authorization, expiration_time, None)
            .map_err(|e| {
                AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    anyhow!("Failed to generate token: {}", e),
                )
            })?;
        Some(token)
    } else {
        None
    };

    let (url, base_url) = generate_context_aware_urls(
        &server_state.url,
        &server_state.allowed_hosts,
        &host.to_string(),
        &doc_id,
    )?;

    Ok(Json(ClientToken {
        url,
        base_url: Some(base_url),
        doc_id,
        token,
        authorization,
    }))
}

fn get_token_from_header(
    auth_header: Option<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>,
) -> Option<String> {
    if let Some(TypedHeader(headers::Authorization(bearer))) = auth_header {
        Some(bearer.token().to_string())
    } else {
        None
    }
}

async fn handle_file_upload_url(
    State(server_state): State<Arc<Server>>,
    Path(doc_id): Path<String>,
    TypedHeader(host): TypedHeader<headers::Host>,
    auth_header: Option<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>,
) -> Result<Json<FileUploadUrlResponse>, AppError> {
    tracing::info!(doc_id = %doc_id, "Generating file upload URL");

    // Get token and extract metadata
    let token = get_token_from_header(auth_header);

    // Verify that the token is for the requested document and extract file hash from token
    if let Some(authenticator) = &server_state.authenticator {
        if let Some(token) = token.as_deref() {
            // Verify token is for this doc_id
            let auth = authenticator
                .verify_file_token_for_doc(token, &doc_id, current_time_epoch_millis())
                .map_err(|e| AppError(StatusCode::UNAUTHORIZED, anyhow!("Invalid token: {}", e)))?;

            // Only allow Full permission to upload
            if !matches!(auth, Authorization::Full) {
                return Err(AppError(
                    StatusCode::FORBIDDEN,
                    anyhow!("Insufficient permissions to upload files"),
                ));
            }

            // Verify the token and get the file metadata
            let permission = authenticator
                .verify_token_auto(token, current_time_epoch_millis())
                .map_err(|_| AppError(StatusCode::UNAUTHORIZED, anyhow!("Invalid token")))?;

            if let Permission::File(file_permission) = permission {
                let file_hash = file_permission.file_hash;

                // Validate the file hash
                if !validate_file_hash(&file_hash) {
                    return Err(AppError(
                        StatusCode::BAD_REQUEST,
                        anyhow!("Invalid file hash format in token"),
                    ));
                }

                // Check if we have a store configured
                if server_state.store.is_none() {
                    return Err(AppError(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        anyhow!("No store configured for file uploads"),
                    ));
                }

                // Get metadata from token
                let content_type = file_permission.content_type.as_deref();
                let content_length = file_permission.content_length;

                // Generate the upload URL - organize files by doc_id/file_hash
                let key = format!("files/{}/{}", doc_id, file_hash);
                let upload_url = server_state
                    .store
                    .as_ref()
                    .unwrap()
                    .generate_upload_url(&key, content_type, content_length)
                    .await
                    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.into()))?;

                if let Some(url) = upload_url {
                    // Check if this is a local endpoint (relative path) and convert to full URL with token
                    if !url.starts_with("http") {
                        let base_url = generate_base_url(
                            &server_state.url,
                            &server_state.allowed_hosts,
                            &host.to_string(),
                        )?;
                        let full_url = format!("{}{}?token={}", base_url, url, token);
                        return Ok(Json(FileUploadUrlResponse {
                            upload_url: full_url,
                        }));
                    } else {
                        // S3/cloud storage URL - return as-is
                        return Ok(Json(FileUploadUrlResponse { upload_url: url }));
                    }
                } else {
                    return Err(AppError(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        anyhow!("Failed to generate upload URL"),
                    ));
                }
            } else {
                return Err(AppError(
                    StatusCode::BAD_REQUEST,
                    anyhow!("Token is not a file token"),
                ));
            }
        } else {
            return Err(AppError(
                StatusCode::UNAUTHORIZED,
                anyhow!("No token provided"),
            ));
        }
    } else {
        // No auth configured, anyone can upload
        return Err(AppError(
            StatusCode::UNAUTHORIZED,
            anyhow!("Authentication is required for file operations"),
        ));
    }
}

async fn handle_file_download_url(
    State(server_state): State<Arc<Server>>,
    Path(doc_id): Path<String>,
    TypedHeader(host): TypedHeader<headers::Host>,
    Query(params): Query<FileDownloadQueryParams>,
    auth_header: Option<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>,
) -> Result<Json<FileDownloadUrlResponse>, AppError> {
    tracing::info!(doc_id = %doc_id, hash = ?params.hash, "Generating file download URL");

    // Get token
    let token = get_token_from_header(auth_header);

    // Check if we have authentication configured
    if let Some(authenticator) = &server_state.authenticator {
        if let Some(token) = token.as_deref() {
            // Extract hash from query parameter if present
            let query_hash = params.hash;

            // Verify the token and determine its type
            let permission = authenticator
                .verify_token_auto(token, current_time_epoch_millis())
                .map_err(|_| AppError(StatusCode::UNAUTHORIZED, anyhow!("Invalid token")))?;

            match permission {
                Permission::File(file_permission) => {
                    // Check if file token is for this doc_id
                    if file_permission.doc_id != doc_id {
                        return Err(AppError(
                            StatusCode::UNAUTHORIZED,
                            anyhow!("Token not valid for this document"),
                        ));
                    }

                    // Both ReadOnly and Full can download files
                    if !matches!(
                        file_permission.authorization,
                        Authorization::ReadOnly | Authorization::Full
                    ) {
                        return Err(AppError(
                            StatusCode::FORBIDDEN,
                            anyhow!("Insufficient permissions to download file"),
                        ));
                    }

                    let file_hash = file_permission.file_hash;

                    // Validate the file hash
                    if !validate_file_hash(&file_hash) {
                        return Err(AppError(
                            StatusCode::BAD_REQUEST,
                            anyhow!("Invalid file hash format in token"),
                        ));
                    }

                    // Generate download URL using hash from token
                    let Json(download_response) = generate_file_download_url(
                        &server_state,
                        &doc_id,
                        &file_hash,
                        &host.to_string(),
                    )
                    .await?;
                    // Add token to the URL
                    let mut download_url = download_response.download_url;
                    if !download_url.starts_with("http") || download_url.contains("/f/") {
                        // This is our local endpoint, add token
                        let separator = if download_url.contains('?') { "&" } else { "?" };
                        download_url = format!("{}{}token={}", download_url, separator, token);
                    }
                    return Ok(Json(FileDownloadUrlResponse { download_url }));
                }
                Permission::Server => {
                    // Server token is valid, use hash from query parameter
                    if let Some(hash) = query_hash {
                        // Validate the file hash from query parameter
                        if !validate_file_hash(&hash) {
                            return Err(AppError(
                                StatusCode::BAD_REQUEST,
                                anyhow!("Invalid file hash format in query parameter"),
                            ));
                        }

                        // Generate download URL using hash from query parameter
                        let Json(download_response) = generate_file_download_url(
                            &server_state,
                            &doc_id,
                            &hash,
                            &host.to_string(),
                        )
                        .await?;
                        // Add file token to the URL (not the server token)
                        let mut download_url = download_response.download_url;
                        if !download_url.starts_with("http") || download_url.contains("/f/") {
                            // This is our local endpoint, generate a proper file token
                            let expiration_time = ExpirationTimeEpochMillis(
                                current_time_epoch_millis() + DEFAULT_EXPIRATION_SECONDS * 1000,
                            );
                            let file_token = authenticator
                                .gen_file_token_auto(
                                    &hash,
                                    &doc_id,
                                    Authorization::Full,
                                    expiration_time,
                                    None,
                                    None,
                                    None,
                                )
                                .map_err(|e| {
                                    AppError(
                                        StatusCode::INTERNAL_SERVER_ERROR,
                                        anyhow!("Failed to generate file token: {}", e),
                                    )
                                })?;
                            let separator = if download_url.contains('?') { "&" } else { "?" };
                            download_url = format!("{}{}token={}", download_url, separator, file_token);
                        }
                        return Ok(Json(FileDownloadUrlResponse { download_url }));
                    } else {
                        return Err(AppError(
                            StatusCode::BAD_REQUEST,
                            anyhow!("Hash query parameter required when using server token"),
                        ));
                    }
                }
                Permission::Doc(_) => {
                    return Err(AppError(
                        StatusCode::BAD_REQUEST,
                        anyhow!("Document tokens cannot be used for file operations"),
                    ));
                }
                Permission::Prefix(prefix_perm) => {
                    // Check if doc_id matches the prefix
                    if !doc_id.starts_with(&prefix_perm.prefix) {
                        return Err(AppError(
                            StatusCode::FORBIDDEN,
                            anyhow!("Token not valid for this document"),
                        ));
                    }

                    // Both ReadOnly and Full can download files
                    if !matches!(
                        prefix_perm.authorization,
                        Authorization::ReadOnly | Authorization::Full
                    ) {
                        return Err(AppError(
                            StatusCode::FORBIDDEN,
                            anyhow!("Insufficient permissions to download file"),
                        ));
                    }

                    // Use hash from query parameter for prefix tokens
                    if let Some(hash) = query_hash {
                        // Validate the file hash from query parameter
                        if !validate_file_hash(&hash) {
                            return Err(AppError(
                                StatusCode::BAD_REQUEST,
                                anyhow!("Invalid file hash format in query parameter"),
                            ));
                        }

                        // Generate download URL using hash from query parameter
                        let Json(download_response) = generate_file_download_url(
                            &server_state,
                            &doc_id,
                            &hash,
                            &host.to_string(),
                        )
                        .await?;
                        // Add file token to the URL (not the prefix token)
                        let mut download_url = download_response.download_url;
                        if !download_url.starts_with("http") || download_url.contains("/f/") {
                            // This is our local endpoint, generate a proper file token
                            let expiration_time = ExpirationTimeEpochMillis(
                                current_time_epoch_millis() + DEFAULT_EXPIRATION_SECONDS * 1000,
                            );
                            let file_token = authenticator
                                .gen_file_token_auto(
                                    &hash,
                                    &doc_id,
                                    prefix_perm.authorization,
                                    expiration_time,
                                    None,
                                    None,
                                    None,
                                )
                                .map_err(|e| {
                                    AppError(
                                        StatusCode::INTERNAL_SERVER_ERROR,
                                        anyhow!("Failed to generate file token: {}", e),
                                    )
                                })?;
                            let separator = if download_url.contains('?') { "&" } else { "?" };
                            download_url = format!("{}{}token={}", download_url, separator, file_token);
                        }
                        return Ok(Json(FileDownloadUrlResponse { download_url }));
                    } else {
                        return Err(AppError(
                            StatusCode::BAD_REQUEST,
                            anyhow!("Hash query parameter required when using prefix token"),
                        ));
                    }
                }
            }
        } else {
            return Err(AppError(
                StatusCode::UNAUTHORIZED,
                anyhow!("No token provided"),
            ));
        }
    } else {
        // No auth configured
        return Err(AppError(
            StatusCode::UNAUTHORIZED,
            anyhow!("Authentication is required for file operations"),
        ));
    }
}

async fn generate_file_download_url(
    server_state: &Arc<Server>,
    doc_id: &str,
    file_hash: &str,
    host: &str,
) -> Result<Json<FileDownloadUrlResponse>, AppError> {
    // Check if we have a store configured
    if server_state.store.is_none() {
        return Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            anyhow!("No store configured for file downloads"),
        ));
    }

    // Generate the download URL - using doc_id/file_hash path structure
    let key = format!("files/{}/{}", doc_id, file_hash);
    let download_url = server_state
        .store
        .as_ref()
        .unwrap()
        .generate_download_url(&key)
        .await
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.into()))?;

    if let Some(url) = download_url {
        // Check if this is a local endpoint (relative path) and convert to full URL
        if !url.starts_with("http") {
            let base_url = generate_base_url(&server_state.url, &server_state.allowed_hosts, host)?;
            let full_url = format!("{}{}", base_url, url);
            Ok(Json(FileDownloadUrlResponse {
                download_url: full_url,
            }))
        } else {
            // S3/cloud storage URL - return as-is
            Ok(Json(FileDownloadUrlResponse { download_url: url }))
        }
    } else {
        Err(AppError(StatusCode::NOT_FOUND, anyhow!("File not found")))
    }
}

/// Delete all files for a document
///
/// This endpoint accepts either:
/// - A file token with the doc_id (hash not required)
/// - A doc token with the doc_id
/// - A server token
///
/// Returns 204 No Content on success
async fn handle_file_delete(
    State(server_state): State<Arc<Server>>,
    Path(doc_id): Path<String>,
    auth_header: Option<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>,
) -> Result<StatusCode, AppError> {
    // Get token
    let token = get_token_from_header(auth_header);

    // Verify token is for this doc_id and has required permission
    if let Some(authenticator) = &server_state.authenticator {
        if let Some(token) = token.as_deref() {
            // Verify token is for this doc_id
            let auth = authenticator
                .verify_file_token_for_doc(token, &doc_id, current_time_epoch_millis())
                .map_err(|e| AppError(StatusCode::UNAUTHORIZED, anyhow!("Invalid token: {}", e)))?;

            // Only Full permission can delete files
            if !matches!(auth, Authorization::Full) {
                return Err(AppError(
                    StatusCode::FORBIDDEN,
                    anyhow!("Insufficient permissions to delete files"),
                ));
            }

            // Check if we have a store configured
            if server_state.store.is_none() {
                return Err(AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    anyhow!("No store configured for file operations"),
                ));
            }

            // List all files in the document's directory
            let prefix = format!("files/{}/", doc_id);
            let store = server_state.store.as_ref().unwrap();

            let file_infos = store
                .list(&prefix)
                .await
                .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.into()))?;

            if file_infos.is_empty() {
                tracing::info!("No files to delete for document: {}", doc_id);
                return Ok(StatusCode::NO_CONTENT);
            }

            // Delete each file
            let mut deleted_count = 0;
            for file_info in file_infos {
                let key = file_info.key;
                if let Err(e) = store.remove(&format!("files/{}/{}", doc_id, key)).await {
                    tracing::error!("Failed to delete file {}/{}: {}", doc_id, key, e);
                    continue;
                }
                deleted_count += 1;
            }

            tracing::info!("Deleted {} files for document: {}", deleted_count, doc_id);
            return Ok(StatusCode::NO_CONTENT);
        } else {
            return Err(AppError(
                StatusCode::UNAUTHORIZED,
                anyhow!("No token provided"),
            ));
        }
    } else {
        // No auth configured
        return Err(AppError(
            StatusCode::UNAUTHORIZED,
            anyhow!("Authentication is required for file operations"),
        ));
    }
}

/// Delete a specific file by hash
///
/// This endpoint accepts either:
/// - A file token with the doc_id (hash not required)
/// - A doc token with the doc_id
/// - A server token
///
/// The hash to delete is specified in the URL path.
/// Returns 204 No Content on success, 404 if file not found
async fn handle_file_delete_by_hash(
    State(server_state): State<Arc<Server>>,
    Path((doc_id, file_hash)): Path<(String, String)>,
    auth_header: Option<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>,
) -> Result<StatusCode, AppError> {
    // Get token
    let token = get_token_from_header(auth_header);

    // Verify token is for this doc_id and has required permission
    if let Some(authenticator) = &server_state.authenticator {
        if let Some(token) = token.as_deref() {
            // Verify token is for this doc_id
            let auth = authenticator
                .verify_file_token_for_doc(token, &doc_id, current_time_epoch_millis())
                .map_err(|e| AppError(StatusCode::UNAUTHORIZED, anyhow!("Invalid token: {}", e)))?;

            // Only Full permission can delete files
            if !matches!(auth, Authorization::Full) {
                return Err(AppError(
                    StatusCode::FORBIDDEN,
                    anyhow!("Insufficient permissions to delete file"),
                ));
            }

            // Validate the file hash format
            if !validate_file_hash(&file_hash) {
                return Err(AppError(
                    StatusCode::BAD_REQUEST,
                    anyhow!("Invalid file hash format"),
                ));
            }

            // Check if we have a store configured
            if server_state.store.is_none() {
                return Err(AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    anyhow!("No store configured for file operations"),
                ));
            }

            // Construct the file path
            let key = format!("files/{}/{}", doc_id, file_hash);

            // Check if the file exists before trying to delete it
            let exists = server_state
                .store
                .as_ref()
                .unwrap()
                .exists(&key)
                .await
                .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.into()))?;

            if !exists {
                // If the file is already gone, return 204 No Content since DELETE is idempotent
                tracing::debug!("File already deleted: {}/{}", doc_id, file_hash);
                return Ok(StatusCode::NO_CONTENT);
            }

            // Delete the file
            server_state
                .store
                .as_ref()
                .unwrap()
                .remove(&key)
                .await
                .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.into()))?;

            tracing::info!("Deleted file: {}/{}", doc_id, file_hash);
            return Ok(StatusCode::NO_CONTENT);
        } else {
            return Err(AppError(
                StatusCode::UNAUTHORIZED,
                anyhow!("No token provided"),
            ));
        }
    } else {
        // No auth configured
        return Err(AppError(
            StatusCode::UNAUTHORIZED,
            anyhow!("Authentication is required for file operations"),
        ));
    }
}

/// Handle HEAD request to check if a file exists in S3 storage
///
/// Returns:
/// - 200 OK if the file exists
/// - 404 Not Found if the file doesn't exist
/// - Other status codes for authentication/authorization errors

/// Get the history of all files for a document
///
/// This endpoint accepts either:
/// - A file token with the doc_id (hash not required)
/// - A doc token with the doc_id
/// - A server token
async fn handle_file_history(
    State(server_state): State<Arc<Server>>,
    Path(doc_id): Path<String>,
    auth_header: Option<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>,
) -> Result<Json<FileHistoryResponse>, AppError> {
    // Get token
    let token = get_token_from_header(auth_header);

    // Verify token is for this doc_id
    if let Some(authenticator) = &server_state.authenticator {
        if let Some(token) = token.as_deref() {
            // Verify token is for this doc_id - this now accepts both doc and file tokens
            let auth = authenticator
                .verify_file_token_for_doc(token, &doc_id, current_time_epoch_millis())
                .map_err(|e| AppError(StatusCode::UNAUTHORIZED, anyhow!("Invalid token: {}", e)))?;

            // Both ReadOnly and Full can view file history
            if !matches!(auth, Authorization::ReadOnly | Authorization::Full) {
                return Err(AppError(
                    StatusCode::FORBIDDEN,
                    anyhow!("Insufficient permissions to view file history"),
                ));
            }
        } else {
            return Err(AppError(
                StatusCode::UNAUTHORIZED,
                anyhow!("No token provided"),
            ));
        }
    }

    // Check if we have a store configured
    if server_state.store.is_none() {
        return Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            anyhow!("No store configured for file operations"),
        ));
    }

    // List files in the document's directory
    let prefix = format!("files/{}/", doc_id);
    let store = server_state.store.as_ref().unwrap();

    let file_infos = store
        .list(&prefix)
        .await
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.into()))?;

    // Convert the raw file info into the API response format
    let files = file_infos
        .into_iter()
        .map(|info| FileHistoryEntry {
            hash: info.key,
            size: info.size,
            created_at: info.last_modified,
        })
        .collect();

    Ok(Json(FileHistoryResponse { files }))
}

async fn handle_doc_versions(
    State(server_state): State<Arc<Server>>,
    Path(doc_id): Path<String>,
    auth_header: Option<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>,
) -> Result<Json<DocumentVersionResponse>, AppError> {
    let token = get_token_from_header(auth_header);

    if let Some(authenticator) = &server_state.authenticator {
        if let Some(token) = token.as_deref() {
            let auth = authenticator
                .verify_doc_token(token, &doc_id, current_time_epoch_millis())
                .map_err(|e| AppError(StatusCode::UNAUTHORIZED, anyhow!("Invalid token: {}", e)))?;

            if !matches!(auth, Authorization::ReadOnly | Authorization::Full) {
                return Err(AppError(
                    StatusCode::FORBIDDEN,
                    anyhow!("Insufficient permissions to view document versions"),
                ));
            }
        } else {
            return Err(AppError(
                StatusCode::UNAUTHORIZED,
                anyhow!("No token provided"),
            ));
        }
    }

    let store = match &server_state.store {
        Some(s) => s,
        None => {
            return Err(AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                anyhow!("No store configured for operations"),
            ))
        }
    };

    let key = format!("{}/data.ysweet", doc_id);
    let versions = store
        .list_versions(&key)
        .await
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.into()))?;

    let entries = versions
        .into_iter()
        .map(|v| DocumentVersionEntry {
            version_id: v.version_id,
            created_at: v.last_modified,
            is_latest: v.is_latest,
        })
        .collect();

    Ok(Json(DocumentVersionResponse { versions: entries }))
}

async fn handle_file_head(
    State(server_state): State<Arc<Server>>,
    Path(doc_id): Path<String>,
    auth_header: Option<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>,
) -> Result<StatusCode, AppError> {
    // Get token
    let token = get_token_from_header(auth_header);

    // Verify token is for this doc_id
    if let Some(authenticator) = &server_state.authenticator {
        if let Some(token) = token.as_deref() {
            // Verify token is for this doc_id
            let auth = authenticator
                .verify_file_token_for_doc(token, &doc_id, current_time_epoch_millis())
                .map_err(|e| AppError(StatusCode::UNAUTHORIZED, anyhow!("Invalid token: {}", e)))?;

            // Both ReadOnly and Full can check if a file exists
            if !matches!(auth, Authorization::ReadOnly | Authorization::Full) {
                return Err(AppError(
                    StatusCode::FORBIDDEN,
                    anyhow!("Insufficient permissions to access file"),
                ));
            }

            // Verify the token and get the file hash
            let permission = authenticator
                .verify_token_auto(token, current_time_epoch_millis())
                .map_err(|_| AppError(StatusCode::UNAUTHORIZED, anyhow!("Invalid token")))?;

            if let Permission::File(file_permission) = permission {
                let file_hash = file_permission.file_hash;

                // Validate the file hash
                if !validate_file_hash(&file_hash) {
                    return Err(AppError(
                        StatusCode::BAD_REQUEST,
                        anyhow!("Invalid file hash format in token"),
                    ));
                }

                // Check if we have a store configured
                if server_state.store.is_none() {
                    return Err(AppError(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        anyhow!("No store configured for file operations"),
                    ));
                }

                // Construct the file path with proper format - using doc_id/file_hash
                let key = format!("files/{}/{}", doc_id, file_hash);

                // Check if the file exists with a direct call to S3
                let exists = server_state
                    .store
                    .as_ref()
                    .unwrap()
                    .exists(&key)
                    .await
                    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.into()))?;

                if exists {
                    tracing::debug!("File exists: {}/{}", doc_id, file_hash);
                    return Ok(StatusCode::OK);
                } else {
                    tracing::debug!("File not found: {}/{}", doc_id, file_hash);
                    return Err(AppError(StatusCode::NOT_FOUND, anyhow!("File not found")));
                }
            } else {
                return Err(AppError(
                    StatusCode::BAD_REQUEST,
                    anyhow!("Token is not a file token"),
                ));
            }
        } else {
            return Err(AppError(
                StatusCode::UNAUTHORIZED,
                anyhow!("No token provided"),
            ));
        }
    } else {
        // No auth configured
        return Err(AppError(
            StatusCode::UNAUTHORIZED,
            anyhow!("Authentication is required for file operations"),
        ));
    }
}

async fn reload_webhook_config_endpoint(
    State(server_state): State<Arc<Server>>,
    auth_header: Option<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>,
) -> Result<Json<Value>, AppError> {
    // Get token
    let token = get_token_from_header(auth_header);

    // Verify token is server token (for server admin operations)
    if let Some(authenticator) = &server_state.authenticator {
        if let Some(token) = token.as_deref() {
            // Verify this is a server admin token
            authenticator
                .verify_server_token(token, current_time_epoch_millis())
                .map_err(|e| AppError(StatusCode::UNAUTHORIZED, anyhow!("Invalid token: {}", e)))?;
        } else {
            return Err(AppError(
                StatusCode::UNAUTHORIZED,
                anyhow!("No token provided"),
            ));
        }
    }

    // Reload webhook configuration
    match server_state.reload_webhook_config().await {
        Ok(status) => Ok(Json(json!({
            "status": "success",
            "message": status
        }))),
        Err(e) => {
            tracing::error!("Failed to reload webhook config: {}", e);
            Err(AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                anyhow!("Failed to reload webhook configuration: {}", e),
            ))
        }
    }
}

async fn metrics_endpoint(State(_server_state): State<Arc<Server>>) -> Result<String, AppError> {
    use prometheus::{Encoder, TextEncoder};

    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();

    encoder.encode(&metric_families, &mut buffer).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            anyhow!("Failed to encode metrics: {}", e),
        )
    })?;

    Ok(String::from_utf8(buffer).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            anyhow!("Failed to convert metrics to string: {}", e),
        )
    })?)
}

#[cfg(test)]
mod test {
    use super::*;
    use y_sweet_core::api_types::Authorization;
    use y_sweet_core::auth::ExpirationTimeEpochMillis;

    #[tokio::test]
    async fn test_auth_doc() {
        let server_state = Server::new(
            None,
            Duration::from_secs(60),
            None,
            None,
            vec![],
            CancellationToken::new(),
            true,
            None,
        )
        .await
        .unwrap();

        let doc_id = server_state.create_doc().await.unwrap();

        let token = auth_doc(
            None,
            TypedHeader(headers::Host::from(http::uri::Authority::from_static(
                "localhost",
            ))),
            State(Arc::new(server_state)),
            Path(doc_id.clone()),
            Some(Json(AuthDocRequest {
                authorization: Authorization::Full,
                user_id: None,
                valid_for_seconds: None,
            })),
        )
        .await
        .unwrap();

        let expected_url = format!("ws://localhost/d/{doc_id}/ws");
        assert_eq!(token.url, expected_url);
        assert_eq!(token.doc_id, doc_id);
        assert!(token.token.is_none());
    }

    #[tokio::test]
    async fn test_auth_doc_with_prefix() {
        let prefix: Url = "https://foo.bar".parse().unwrap();
        let server_state = Server::new(
            None,
            Duration::from_secs(60),
            None,
            Some(prefix),
            vec![],
            CancellationToken::new(),
            true,
            None,
        )
        .await
        .unwrap();

        let doc_id = server_state.create_doc().await.unwrap();

        let token = auth_doc(
            None,
            TypedHeader(headers::Host::from(http::uri::Authority::from_static(
                "localhost",
            ))),
            State(Arc::new(server_state)),
            Path(doc_id.clone()),
            None,
        )
        .await
        .unwrap();

        let expected_url = format!("wss://foo.bar/d/{doc_id}/ws");
        assert_eq!(token.url, expected_url);
        assert_eq!(token.doc_id, doc_id);
        assert!(token.token.is_none());
    }

    #[tokio::test]
    async fn test_file_head_endpoint() {
        use async_trait::async_trait;
        use std::collections::HashMap;
        use std::sync::Arc;
        use y_sweet_core::store::Result as StoreResult;

        // Create a mock store for testing
        #[derive(Clone)]
        struct MockStore {
            files: Arc<HashMap<String, Vec<u8>>>,
        }

        #[async_trait]
        impl Store for MockStore {
            async fn init(&self) -> StoreResult<()> {
                Ok(())
            }

            async fn get(&self, key: &str) -> StoreResult<Option<Vec<u8>>> {
                Ok(self.files.get(key).cloned())
            }

            async fn set(&self, _key: &str, _value: Vec<u8>) -> StoreResult<()> {
                Ok(())
            }

            async fn remove(&self, _key: &str) -> StoreResult<()> {
                Ok(())
            }

            async fn exists(&self, key: &str) -> StoreResult<bool> {
                Ok(self.files.contains_key(key))
            }

            async fn generate_upload_url(
                &self,
                _key: &str,
                _content_type: Option<&str>,
                _content_length: Option<u64>,
            ) -> StoreResult<Option<String>> {
                Ok(Some("http://mock-upload-url".to_string()))
            }

            async fn generate_download_url(&self, _key: &str) -> StoreResult<Option<String>> {
                Ok(Some("http://mock-download-url".to_string()))
            }
        }

        // Create a mock authenticator
        let mut authenticator = y_sweet_core::auth::Authenticator::gen_key().unwrap();
        authenticator.set_expected_audience(Some("https://api.example.com".to_string()));
        let doc_id = "test-doc-123";
        let file_hash = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";

        // Generate a file token
        let token = authenticator
            .gen_file_token_cwt(
                file_hash,
                doc_id,
                Authorization::Full,
                ExpirationTimeEpochMillis(u64::MAX), // Never expires for test
                None,
                None,
                None,
                None, // channel
            )
            .unwrap();

        // Set up the mock store with the test file
        let mut mock_files = HashMap::new();
        mock_files.insert(format!("files/{}/{}", doc_id, file_hash), vec![1, 2, 3, 4]);

        let mock_store = MockStore {
            files: Arc::new(mock_files),
        };

        // Create the server with our mock components
        let server_state = Arc::new(
            Server::new(
                Some(Box::new(mock_store)),
                Duration::from_secs(60),
                Some(authenticator.clone()),
                None,
                vec![],
                CancellationToken::new(),
                true,
                None,
            )
            .await
            .unwrap(),
        );

        // Create auth header with token
        let headers = TypedHeader(headers::Authorization::bearer(&token).unwrap());

        // Test the HEAD endpoint - should return 200 OK for existing file
        let result = handle_file_head(
            State(server_state.clone()),
            Path(doc_id.to_string()),
            Some(headers.clone()),
        )
        .await;

        assert!(
            result.is_ok(),
            "HEAD request should succeed for existing file"
        );
        assert_eq!(result.unwrap(), StatusCode::OK);

        // Test a file that doesn't exist
        let nonexistent_file_hash =
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        let nonexistent_token = authenticator
            .gen_file_token_cwt(
                nonexistent_file_hash,
                doc_id,
                Authorization::Full,
                ExpirationTimeEpochMillis(u64::MAX),
                None,
                None,
                None,
                None, // channel
            )
            .unwrap();

        let nonexistent_headers =
            TypedHeader(headers::Authorization::bearer(&nonexistent_token).unwrap());

        let result = handle_file_head(
            State(server_state),
            Path(doc_id.to_string()),
            Some(nonexistent_headers),
        )
        .await;

        assert!(
            result.is_err(),
            "HEAD request should fail for non-existent file"
        );
        match result {
            Err(AppError(status, _)) => assert_eq!(status, StatusCode::NOT_FOUND),
            _ => panic!("Expected NOT_FOUND status for non-existent file"),
        };
    }

    #[tokio::test]
    async fn test_generate_context_aware_urls_with_prefix() {
        let url: Url = "https://api.example.com".parse().unwrap();
        let allowed_hosts = vec![];
        let doc_id = "test-doc";

        let (ws_url, base_url) =
            generate_context_aware_urls(&Some(url), &allowed_hosts, "unused-host", doc_id).unwrap();

        assert_eq!(ws_url, "wss://api.example.com/d/test-doc/ws");
        assert_eq!(base_url, "https://api.example.com/d/test-doc");
    }

    #[tokio::test]
    async fn test_generate_context_aware_urls_with_allowed_hosts() {
        let allowed_hosts = vec![
            AllowedHost {
                host: "api.example.com".to_string(),
                scheme: "https".to_string(),
            },
            AllowedHost {
                host: "app.flycast".to_string(),
                scheme: "http".to_string(),
            },
        ];
        let doc_id = "test-doc";

        // Test HTTPS host
        let (ws_url, base_url) =
            generate_context_aware_urls(&None, &allowed_hosts, "api.example.com", doc_id).unwrap();

        assert_eq!(ws_url, "wss://api.example.com/d/test-doc/ws");
        assert_eq!(base_url, "https://api.example.com/d/test-doc");

        // Test flycast host
        let (ws_url, base_url) =
            generate_context_aware_urls(&None, &allowed_hosts, "app.flycast", doc_id).unwrap();

        assert_eq!(ws_url, "ws://app.flycast/d/test-doc/ws");
        assert_eq!(base_url, "http://app.flycast/d/test-doc");
    }

    #[tokio::test]
    async fn test_generate_context_aware_urls_rejects_unknown_host() {
        let allowed_hosts = vec![AllowedHost {
            host: "api.example.com".to_string(),
            scheme: "https".to_string(),
        }];
        let doc_id = "test-doc";

        let result = generate_context_aware_urls(&None, &allowed_hosts, "malicious.host", doc_id);

        assert!(result.is_err());
        match result {
            Err(AppError(StatusCode::BAD_REQUEST, _)) => {} // Expected
            _ => panic!("Expected BAD_REQUEST for unknown host"),
        }
    }

    #[tokio::test]
    async fn test_auth_doc_with_context_aware_urls() {
        let allowed_hosts = vec![
            AllowedHost {
                host: "api.example.com".to_string(),
                scheme: "https".to_string(),
            },
            AllowedHost {
                host: "app.flycast".to_string(),
                scheme: "http".to_string(),
            },
        ];

        let server_state = Arc::new(
            Server::new(
                None,
                Duration::from_secs(60),
                None,
                None, // No URL prefix - use context-aware generation
                allowed_hosts.clone(),
                CancellationToken::new(),
                true,
                None,
            )
            .await
            .unwrap(),
        );

        let doc_id = server_state.create_doc().await.unwrap();

        // Test with HTTPS host
        let token = auth_doc(
            None,
            TypedHeader(headers::Host::from(http::uri::Authority::from_static(
                "api.example.com",
            ))),
            State(server_state.clone()),
            Path(doc_id.clone()),
            Some(Json(AuthDocRequest {
                authorization: Authorization::Full,
                user_id: None,
                valid_for_seconds: None,
            })),
        )
        .await
        .unwrap();

        assert_eq!(token.url, format!("wss://api.example.com/d/{}/ws", doc_id));
        assert_eq!(
            token.base_url,
            Some(format!("https://api.example.com/d/{}", doc_id))
        );

        // Test with flycast host - create another server instance with same allowed hosts
        let server_state2 = Arc::new(
            Server::new(
                None,
                Duration::from_secs(60),
                None,
                None,
                allowed_hosts,
                CancellationToken::new(),
                true,
                None,
            )
            .await
            .unwrap(),
        );

        server_state2.load_doc(&doc_id, None).await.unwrap();

        let token = auth_doc(
            None,
            TypedHeader(headers::Host::from(http::uri::Authority::from_static(
                "app.flycast",
            ))),
            State(server_state2),
            Path(doc_id.clone()),
            Some(Json(AuthDocRequest {
                authorization: Authorization::Full,
                user_id: None,
                valid_for_seconds: None,
            })),
        )
        .await
        .unwrap();

        assert_eq!(token.url, format!("ws://app.flycast/d/{}/ws", doc_id));
        assert_eq!(
            token.base_url,
            Some(format!("http://app.flycast/d/{}", doc_id))
        );
    }

    #[tokio::test]
    async fn test_file_upload_url_with_filesystem_store() {
        use crate::stores::filesystem::FileSystemStore;
        use tempfile::TempDir;
        use y_sweet_core::api_types::Authorization;
        use y_sweet_core::auth::{Authenticator, ExpirationTimeEpochMillis};

        // Create a test authenticator
        let mut authenticator = Authenticator::gen_key().unwrap();
        authenticator.set_expected_audience(Some("https://api.example.com".to_string()));

        let allowed_hosts = vec![AllowedHost {
            host: "api.example.com".to_string(),
            scheme: "https".to_string(),
        }];

        // Create filesystem store
        let temp_dir = TempDir::new().unwrap();
        let store = FileSystemStore::new(temp_dir.path().to_path_buf()).unwrap();

        let server_state = Arc::new(
            Server::new(
                Some(Box::new(store)),
                Duration::from_secs(60),
                Some(authenticator.clone()),
                None,
                allowed_hosts,
                CancellationToken::new(),
                true,
                None,
            )
            .await
            .unwrap(),
        );

        let doc_id = "test-doc";
        let file_hash = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";

        // Generate a file token
        let token = authenticator
            .gen_file_token_cwt(
                file_hash,
                doc_id,
                Authorization::Full,
                ExpirationTimeEpochMillis(u64::MAX),
                Some("image/png"),
                Some(1024),
                None,
                None,
            )
            .unwrap();

        // Test upload URL generation
        let host_header = TypedHeader(headers::Host::from(http::uri::Authority::from_static(
            "api.example.com",
        )));
        let auth_header = Some(TypedHeader(headers::Authorization::bearer(&token).unwrap()));

        let result = handle_file_upload_url(
            State(server_state),
            Path(doc_id.to_string()),
            host_header,
            auth_header,
        )
        .await
        .unwrap();

        let Json(response) = result;
        // Should get full HTTPS URL with token
        assert!(response
            .upload_url
            .starts_with("https://api.example.com/f/"));
        assert!(response
            .upload_url
            .contains(&format!("/f/{}/upload", doc_id)));
        assert!(response.upload_url.contains(&format!("token={}", token)));
    }

    #[tokio::test]
    async fn test_file_download_url_with_filesystem_store() {
        use crate::stores::filesystem::FileSystemStore;
        use tempfile::TempDir;
        use y_sweet_core::api_types::Authorization;
        use y_sweet_core::auth::{Authenticator, ExpirationTimeEpochMillis};

        // Create a test authenticator
        let mut authenticator = Authenticator::gen_key().unwrap();
        authenticator.set_expected_audience(Some("http://localhost".to_string()));

        let allowed_hosts = vec![AllowedHost {
            host: "localhost".to_string(),
            scheme: "http".to_string(),
        }];

        // Create filesystem store
        let temp_dir = TempDir::new().unwrap();
        let store = FileSystemStore::new(temp_dir.path().to_path_buf()).unwrap();

        let server_state = Arc::new(
            Server::new(
                Some(Box::new(store)),
                Duration::from_secs(60),
                Some(authenticator.clone()),
                None,
                allowed_hosts,
                CancellationToken::new(),
                true,
                None,
            )
            .await
            .unwrap(),
        );

        let doc_id = "test-doc";
        let file_hash = "def456789012345678901234567890def456789012345678901234567890def4";

        // Generate a file token
        let token = authenticator
            .gen_file_token_cwt(
                file_hash,
                doc_id,
                Authorization::ReadOnly,
                ExpirationTimeEpochMillis(u64::MAX),
                Some("image/jpeg"),
                Some(2048),
                None,
                None,
            )
            .unwrap();

        // Test download URL generation
        let host_header = TypedHeader(headers::Host::from(http::uri::Authority::from_static(
            "localhost",
        )));
        let auth_header = Some(TypedHeader(headers::Authorization::bearer(&token).unwrap()));

        let result = handle_file_download_url(
            State(server_state),
            Path(doc_id.to_string()),
            host_header,
            Query(FileDownloadQueryParams { hash: None }),
            auth_header,
        )
        .await
        .unwrap();

        let Json(response) = result;
        // Should get full HTTP URL with hash and token
        assert!(response.download_url.starts_with("http://localhost/f/"));
        assert!(response
            .download_url
            .contains(&format!("/f/{}/download", doc_id)));
        assert!(response
            .download_url
            .contains(&format!("hash={}", file_hash)));
        assert!(response.download_url.contains(&format!("token={}", token)));
    }
}

async fn handle_file_upload(
    State(server_state): State<Arc<Server>>,
    Path(doc_id): Path<String>,
    Query(params): Query<FileUploadParams>,
    mut multipart: Multipart,
) -> Result<StatusCode, AppError> {
    tracing::info!(doc_id = %doc_id, "Handling file upload");

    let permission = validate_file_token(&server_state, &params.token, &doc_id)?;

    if let Permission::File(file_permission) = permission {
        // Only allow Full permission to upload
        if !matches!(file_permission.authorization, Authorization::Full) {
            return Err(AppError(
                StatusCode::FORBIDDEN,
                anyhow!("Insufficient permissions to upload files"),
            ));
        }

        // Get file field from multipart stream
        let field = multipart
            .next_field()
            .await
            .map_err(|e| AppError(StatusCode::BAD_REQUEST, e.into()))?
            .ok_or_else(|| AppError(StatusCode::BAD_REQUEST, anyhow!("No file provided")))?;

        // Validate content-type if specified in token
        if let Some(expected_type) = &file_permission.content_type {
            if field.content_type() != Some(expected_type) {
                return Err(AppError(
                    StatusCode::BAD_REQUEST,
                    anyhow!("Content-Type mismatch: expected {}", expected_type),
                ));
            }
        }

        // Check if we have a store configured
        let store = server_state.store.as_ref().ok_or_else(|| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                anyhow!("No store configured for file uploads"),
            )
        })?;

        // Prepare for streaming validation
        let key = format!("files/{}/{}", doc_id, file_permission.file_hash);

        // Create a temporary file for atomic writes
        let temp_file = NamedTempFile::new()
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.into()))?;

        let mut hasher = Sha256::new();
        let mut total_size = 0u64;
        let mut file_writer = temp_file.as_file();

        // Stream chunks while validating
        let mut stream = field.into_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| AppError(StatusCode::BAD_REQUEST, e.into()))?;

            // Update hash and size
            hasher.update(&chunk);
            total_size += chunk.len() as u64;

            // Early size validation
            if let Some(expected_length) = file_permission.content_length {
                if total_size > expected_length {
                    return Err(AppError(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        anyhow!("File exceeds expected size"),
                    ));
                }
            }

            // Write to temp file
            file_writer
                .write_all(&chunk)
                .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.into()))?;
        }

        // Final validations
        if let Some(expected_length) = file_permission.content_length {
            if total_size != expected_length {
                return Err(AppError(
                    StatusCode::BAD_REQUEST,
                    anyhow!(
                        "Content-Length mismatch: expected {}, got {}",
                        expected_length,
                        total_size
                    ),
                ));
            }
        }

        let actual_hash = format!("{:x}", hasher.finalize());
        if actual_hash != file_permission.file_hash {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                anyhow!(
                    "File hash mismatch: expected {}, got {}",
                    file_permission.file_hash,
                    actual_hash
                ),
            ));
        }

        // Read the temp file contents and store using the store interface
        let file_contents = std::fs::read(temp_file.path())
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.into()))?;

        store
            .set(&key, file_contents)
            .await
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.into()))?;

        Ok(StatusCode::OK)
    } else {
        Err(AppError(
            StatusCode::BAD_REQUEST,
            anyhow!("Invalid permission type"),
        ))
    }
}

async fn handle_file_upload_raw(
    State(server_state): State<Arc<Server>>,
    Path(doc_id): Path<String>,
    Query(params): Query<FileUploadParams>,
    body: axum::body::Bytes,
) -> Result<StatusCode, AppError> {
    tracing::info!(doc_id = %doc_id, "Handling raw file upload");

    let permission = validate_file_token(&server_state, &params.token, &doc_id)?;

    if let Permission::File(file_permission) = permission {
        // Only allow Full permission to upload
        if !matches!(file_permission.authorization, Authorization::Full) {
            return Err(AppError(
                StatusCode::FORBIDDEN,
                anyhow!("Insufficient permissions to upload files"),
            ));
        }

        // Check if we have a store configured
        let store = server_state.store.as_ref().ok_or_else(|| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                anyhow!("No store configured for file uploads"),
            )
        })?;

        let key = format!("files/{}/{}", doc_id, file_permission.file_hash);

        // Validate content length if specified in token
        if let Some(expected_length) = file_permission.content_length {
            if body.len() as u64 != expected_length {
                return Err(AppError(
                    StatusCode::BAD_REQUEST,
                    anyhow!(
                        "Content-Length mismatch: expected {}, got {}",
                        expected_length,
                        body.len()
                    ),
                ));
            }
        }

        // Validate file hash
        let mut hasher = Sha256::new();
        hasher.update(&body);
        let actual_hash = format!("{:x}", hasher.finalize());

        if actual_hash != file_permission.file_hash {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                anyhow!(
                    "File hash mismatch: expected {}, got {}",
                    file_permission.file_hash,
                    actual_hash
                ),
            ));
        }

        // Store the file
        store
            .set(&key, body.to_vec())
            .await
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.into()))?;

        Ok(StatusCode::OK)
    } else {
        Err(AppError(
            StatusCode::BAD_REQUEST,
            anyhow!("Invalid permission type"),
        ))
    }
}

async fn handle_file_download(
    State(server_state): State<Arc<Server>>,
    Path(doc_id): Path<String>,
    Query(params): Query<FileDownloadParams>,
) -> Result<Response, AppError> {
    tracing::info!(doc_id = %doc_id, hash = %params.hash, "Handling file download");

    let permission = validate_file_token(&server_state, &params.token, &doc_id)?;

    if let Permission::File(file_permission) = permission {
        // Both ReadOnly and Full can download files
        if !matches!(
            file_permission.authorization,
            Authorization::ReadOnly | Authorization::Full
        ) {
            return Err(AppError(
                StatusCode::FORBIDDEN,
                anyhow!("Insufficient permissions to download file"),
            ));
        }

        // Verify the hash parameter matches the token
        if file_permission.file_hash != params.hash {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                anyhow!("Hash parameter does not match token"),
            ));
        }

        // Check if we have a store configured
        let store = server_state.store.as_ref().ok_or_else(|| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                anyhow!("No store configured for file downloads"),
            )
        })?;

        // Retrieve file
        let key = format!("files/{}/{}", doc_id, file_permission.file_hash);
        let file_data = store
            .get(&key)
            .await
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.into()))?
            .ok_or_else(|| AppError(StatusCode::NOT_FOUND, anyhow!("File not found")))?;

        // Stream response
        let content_type = file_permission
            .content_type
            .unwrap_or_else(|| "application/octet-stream".to_string());

        Ok(Response::builder()
            .status(StatusCode::OK)
            .header("content-type", content_type)
            .header("content-length", file_data.len())
            .body(axum::body::Body::from(file_data))
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.into()))?)
    } else {
        Err(AppError(
            StatusCode::BAD_REQUEST,
            anyhow!("Invalid permission type"),
        ))
    }
}
