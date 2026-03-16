//! Crossterm event reader with scroll debouncing.

use std::time::{Duration, Instant};

use crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tokio::sync::mpsc;

use super::AppEvent;

/// Handles crossterm event reading and dispatches [`AppEvent`]s.
pub struct EventHandler {
    /// Channel sender for emitting events.
    tx: mpsc::UnboundedSender<AppEvent>,
    /// Channel receiver for consuming events.
    rx: mpsc::UnboundedReceiver<AppEvent>,
    /// Tick rate for periodic updates.
    tick_rate: Duration,
}

impl EventHandler {
    /// Create a new event handler with the given tick rate.
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self { tx, rx, tick_rate }
    }

    /// Get a clone of the sender for external event producers (agent, tools).
    pub fn sender(&self) -> mpsc::UnboundedSender<AppEvent> {
        self.tx.clone()
    }

    /// Start the crossterm event reader loop.
    ///
    /// Uses crossterm's async `EventStream` for zero-latency event delivery
    /// instead of `spawn_blocking` + poll which adds up to 160ms delay.
    ///
    /// Includes a debounce state machine that distinguishes touchpad/mouse scroll
    /// (rapid-fire Up/Down arrows via xterm alternate scroll mode) from keyboard
    /// arrow presses. Touchpad scroll generates arrows every 8-16ms in bursts;
    /// keyboard presses are single events with ~300ms before repeat starts.
    /// A 25ms debounce window cleanly separates these two input sources.
    pub fn start(&self) {
        use futures::StreamExt;
        let tx = self.tx.clone();
        let tick_rate = self.tick_rate;

        tokio::spawn(async move {
            let mut reader = crossterm::event::EventStream::new();
            let mut tick_interval = tokio::time::interval(tick_rate);

            // Debounce state for distinguishing mouse scroll from keyboard arrows
            let debounce_window = Duration::from_millis(25);
            let scroll_burst_timeout = Duration::from_millis(100);
            let mut pending_arrow: Option<(KeyEvent, Instant)> = None;
            let mut scroll_burst = false;
            let mut last_arrow_time: Option<Instant> = None;

            loop {
                // Compute debounce deadline if we have a pending arrow
                let debounce_deadline = pending_arrow
                    .as_ref()
                    .map(|(_, t)| tokio::time::Instant::from_std(*t + debounce_window));

                tokio::select! {
                    biased;

                    // Debounce timer fires — pending arrow was a keyboard press
                    _ = async {
                        match debounce_deadline {
                            Some(deadline) => tokio::time::sleep_until(deadline).await,
                            None => std::future::pending().await,
                        }
                    } => {
                        if let Some((key, _)) = pending_arrow.take() {
                            scroll_burst = false;
                            if tx.send(AppEvent::Key(key)).is_err() {
                                break;
                            }
                        }
                    }

                    maybe_event = reader.next() => {
                        match maybe_event {
                            Some(Ok(CrosstermEvent::Key(key))) => {
                                let is_unmodified_arrow = matches!(
                                    key.code,
                                    KeyCode::Up | KeyCode::Down
                                ) && key.modifiers == KeyModifiers::NONE
                                  && key.kind == KeyEventKind::Press;

                                if is_unmodified_arrow {
                                    let now = Instant::now();

                                    // Check if we're in a scroll burst
                                    let in_burst = scroll_burst
                                        && last_arrow_time.is_some_and(|t| {
                                            now.duration_since(t) < scroll_burst_timeout
                                        });

                                    if let Some((prev_key, _)) = pending_arrow.take() {
                                        // Second arrow arrived within debounce window → mouse scroll
                                        scroll_burst = true;
                                        last_arrow_time = Some(now);
                                        let ev1 = if prev_key.code == KeyCode::Up {
                                            AppEvent::ScrollUp
                                        } else {
                                            AppEvent::ScrollDown
                                        };
                                        let ev2 = if key.code == KeyCode::Up {
                                            AppEvent::ScrollUp
                                        } else {
                                            AppEvent::ScrollDown
                                        };
                                        if tx.send(ev1).is_err() || tx.send(ev2).is_err() {
                                            break;
                                        }
                                    } else if in_burst {
                                        // Continuing a scroll burst
                                        last_arrow_time = Some(now);
                                        let ev = if key.code == KeyCode::Up {
                                            AppEvent::ScrollUp
                                        } else {
                                            AppEvent::ScrollDown
                                        };
                                        if tx.send(ev).is_err() {
                                            break;
                                        }
                                    } else {
                                        // First arrow — buffer it, wait for debounce
                                        pending_arrow = Some((key, now));
                                    }
                                } else {
                                    // Non-arrow key or arrow with modifiers/repeat:
                                    // flush any pending arrow as keyboard first
                                    if let Some((prev_key, _)) = pending_arrow.take() {
                                        scroll_burst = false;
                                        if tx.send(AppEvent::Key(prev_key)).is_err() {
                                            break;
                                        }
                                    }
                                    // Key repeat on arrows bypasses debounce (keyboard hold)
                                    if tx.send(AppEvent::Key(key)).is_err() {
                                        break;
                                    }
                                }
                            }
                            Some(Ok(CrosstermEvent::Mouse(_))) => continue,
                            Some(Ok(CrosstermEvent::Resize(w, h))) => {
                                // Flush pending arrow before resize
                                if let Some((prev_key, _)) = pending_arrow.take() {
                                    scroll_burst = false;
                                    if tx.send(AppEvent::Key(prev_key)).is_err() {
                                        break;
                                    }
                                }
                                if tx.send(AppEvent::Resize(w, h)).is_err() {
                                    break;
                                }
                            }
                            Some(Ok(other)) => {
                                // Flush pending arrow before other events
                                if let Some((prev_key, _)) = pending_arrow.take() {
                                    scroll_burst = false;
                                    if tx.send(AppEvent::Key(prev_key)).is_err() {
                                        break;
                                    }
                                }
                                if tx.send(AppEvent::Terminal(other)).is_err() {
                                    break;
                                }
                            }
                            Some(Err(_)) => continue,
                            None => break,
                        }
                    }

                    _ = tick_interval.tick() => {
                        // Don't flush pending arrow on tick — let debounce timer handle it
                        if tx.send(AppEvent::Tick).is_err() {
                            break;
                        }
                    }
                }
            }
        });
    }

    /// Receive the next event.
    pub async fn next(&mut self) -> Option<AppEvent> {
        self.rx.recv().await
    }

    /// Try to receive an event without blocking.
    /// Returns `None` immediately if no event is queued.
    pub fn try_next(&mut self) -> Option<AppEvent> {
        self.rx.try_recv().ok()
    }
}
