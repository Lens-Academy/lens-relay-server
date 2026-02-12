/**
 * Document load timing instrumentation.
 *
 * Tracks the critical path from navigation click to content visible:
 *   nav → provider-mount → auth-start → auth-end → ws-connecting →
 *   ws-handshaking → ws-connected (synced) → editor-synced
 *
 * Usage: import { loadTimer } from './load-timing' then call loadTimer.mark('step')
 * Results print to console as a table when the final mark fires.
 *
 * Only marks for the tracked docId are recorded — folder metadata
 * providers that share the same auth path are filtered out.
 */

interface TimingEntry {
  label: string;
  time: number;
  delta: number;
}

class LoadTimer {
  private entries: TimingEntry[] = [];
  private origin = 0;
  private trackedDocId: string | null = null;
  private finished = false;

  /**
   * Start a new timing session for a document load.
   * Resets all previous marks.
   */
  start(docId: string) {
    this.entries = [];
    this.origin = performance.now();
    this.trackedDocId = docId;
    this.finished = false;
    this.addEntry('nav');
  }

  /** The docId currently being timed. */
  get activeDocId(): string | null {
    return this.trackedDocId;
  }

  /**
   * Record a timing mark. Delta is computed from the previous mark.
   * Ignored if no session is active or session already finished.
   */
  mark(label: string) {
    if (!this.trackedDocId || this.finished) return;
    this.addEntry(label);
  }

  private addEntry(label: string) {
    const now = performance.now();
    const elapsed = now - this.origin;
    const prev = this.entries.length > 0 ? this.entries[this.entries.length - 1].time : 0;
    this.entries.push({ label, time: elapsed, delta: elapsed - prev });
  }

  /**
   * Mark the final step and print the results table.
   */
  finish(label = 'done') {
    if (this.finished) return;
    this.finished = true;
    this.addEntry(label);
    this.print();
  }

  private print() {
    const total = this.entries[this.entries.length - 1].time;
    const shortId = this.trackedDocId?.slice(-8) ?? '?';

    const lines = this.entries.map(
      (e) => `  ${e.label.padEnd(24)} ${e.time.toFixed(0).padStart(6)}ms  (+${e.delta.toFixed(0)}ms)`
    );
    console.log(
      `[LoadTimer] ...${shortId} total=${total.toFixed(0)}ms\n${lines.join('\n')}`
    );
  }
}

export const loadTimer = new LoadTimer();
