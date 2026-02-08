use dashmap::DashMap;
use serde_json::Value;
use std::time::Instant;

pub struct McpSession {
    pub session_id: String,
    pub protocol_version: String,
    pub client_info: Option<Value>,
    pub initialized: bool,
    pub created_at: Instant,
    pub last_activity: Instant,
}

pub struct SessionManager {
    sessions: DashMap<String, McpSession>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
        }
    }

    /// Create a new session, returning the session ID.
    pub fn create_session(
        &self,
        protocol_version: String,
        client_info: Option<Value>,
    ) -> String {
        let session_id = nanoid::nanoid!(32);
        let now = Instant::now();
        let session = McpSession {
            session_id: session_id.clone(),
            protocol_version,
            client_info,
            initialized: false,
            created_at: now,
            last_activity: now,
        };
        self.sessions.insert(session_id.clone(), session);
        session_id
    }

    /// Look up a session by ID.
    pub fn get_session(
        &self,
        session_id: &str,
    ) -> Option<dashmap::mapref::one::Ref<'_, String, McpSession>> {
        self.sessions.get(session_id)
    }

    /// Get a mutable reference to a session.
    pub fn get_session_mut(
        &self,
        session_id: &str,
    ) -> Option<dashmap::mapref::one::RefMut<'_, String, McpSession>> {
        self.sessions.get_mut(session_id)
    }

    /// Mark a session as initialized. Returns true if session existed.
    pub fn mark_initialized(&self, session_id: &str) -> bool {
        if let Some(mut session) = self.sessions.get_mut(session_id) {
            session.initialized = true;
            session.last_activity = Instant::now();
            true
        } else {
            false
        }
    }

    /// Remove a session. Returns true if session existed.
    pub fn remove_session(&self, session_id: &str) -> bool {
        self.sessions.remove(session_id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn create_session_returns_32_char_id() {
        let mgr = SessionManager::new();
        let id = mgr.create_session("2025-03-26".into(), None);
        assert_eq!(id.len(), 32);
    }

    #[test]
    fn two_sessions_have_different_ids() {
        let mgr = SessionManager::new();
        let id1 = mgr.create_session("2025-03-26".into(), None);
        let id2 = mgr.create_session("2025-03-26".into(), None);
        assert_ne!(id1, id2);
    }

    #[test]
    fn get_session_valid_id() {
        let mgr = SessionManager::new();
        let id = mgr.create_session("2025-03-26".into(), Some(json!({"name": "test"})));
        let session = mgr.get_session(&id).expect("session should exist");
        assert_eq!(session.session_id, id);
        assert_eq!(session.protocol_version, "2025-03-26");
        assert!(!session.initialized);
        assert!(session.client_info.is_some());
    }

    #[test]
    fn get_session_invalid_id() {
        let mgr = SessionManager::new();
        assert!(mgr.get_session("nonexistent").is_none());
    }

    #[test]
    fn mark_initialized_sets_flag() {
        let mgr = SessionManager::new();
        let id = mgr.create_session("2025-03-26".into(), None);

        assert!(!mgr.get_session(&id).unwrap().initialized);
        assert!(mgr.mark_initialized(&id));
        assert!(mgr.get_session(&id).unwrap().initialized);
    }

    #[test]
    fn mark_initialized_nonexistent_returns_false() {
        let mgr = SessionManager::new();
        assert!(!mgr.mark_initialized("nonexistent"));
    }

    #[test]
    fn remove_session_makes_it_inaccessible() {
        let mgr = SessionManager::new();
        let id = mgr.create_session("2025-03-26".into(), None);
        assert!(mgr.get_session(&id).is_some());
        assert!(mgr.remove_session(&id));
        assert!(mgr.get_session(&id).is_none());
    }

    #[test]
    fn remove_nonexistent_session_returns_false() {
        let mgr = SessionManager::new();
        assert!(!mgr.remove_session("nonexistent"));
    }
}
