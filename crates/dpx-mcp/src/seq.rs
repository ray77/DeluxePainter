//! Arrival order for tool calls (ADR-43). rmcp runs every request in a task of its own, so
//! two calls that arrive one after the other may start in either order. A [`Ticket`] is taken
//! when a call *arrives* (the stdio transport's `receive`, the HTTP middleware), and the call
//! runs only in its [`Turn`]: after every call with an earlier ticket has finished or was
//! dropped. Calls are therefore serialised in arrival order, whole: a tool's jobs, file I/O
//! and encoding never interleave with another call's.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::oneshot;

/// Hands out tickets and turns in ticket order.
#[derive(Debug, Default)]
pub struct Sequencer {
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    /// The next ticket number to hand out.
    issued: u64,
    /// The ticket whose turn it is (or comes next).
    next: u64,
    /// Tickets after `next` that are finished or dropped.
    released: BTreeSet<u64>,
    /// Calls waiting for their turn.
    waiting: HashMap<u64, oneshot::Sender<()>>,
}

/// A place in the arrival order. Cheap to clone (it may travel in HTTP request extensions);
/// dropping every clone without taking the turn gives the place up.
#[derive(Clone, Debug)]
pub struct Ticket(Arc<TicketInner>);

#[derive(Debug)]
struct TicketInner {
    seq: u64,
    owner: Arc<Sequencer>,
    /// Set once a turn was taken with this ticket (the turn releases it then).
    used: AtomicBool,
}

impl Drop for TicketInner {
    fn drop(&mut self) {
        if !*self.used.get_mut() {
            self.owner.release(self.seq);
        }
    }
}

/// The right to run now; every later ticket waits until this is dropped.
#[derive(Debug)]
pub struct Turn {
    seq: u64,
    owner: Arc<Sequencer>,
}

impl Drop for Turn {
    fn drop(&mut self) {
        self.owner.release(self.seq);
    }
}

impl Sequencer {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The next place in the arrival order.
    pub fn ticket(self: &Arc<Self>) -> Ticket {
        let mut st = self.lock();
        let seq = st.issued;
        st.issued += 1;
        Ticket(Arc::new(TicketInner {
            seq,
            owner: self.clone(),
            used: AtomicBool::new(false),
        }))
    }

    /// Waits until every earlier ticket is released, then holds the turn. `None` takes a new
    /// ticket now (a call that arrived without one). A ticket whose turn was taken already
    /// (through a clone) also counts as none.
    pub async fn turn(self: &Arc<Self>, ticket: Option<Ticket>) -> Turn {
        let ticket = match ticket {
            Some(t) if !t.0.used.swap(true, Ordering::SeqCst) => t,
            _ => {
                let t = self.ticket();
                t.0.used.store(true, Ordering::SeqCst);
                t
            }
        };
        let seq = ticket.0.seq;
        drop(ticket);
        // From here the turn owns the place: a call dropped while it waits releases it.
        let turn = Turn {
            seq,
            owner: self.clone(),
        };
        let wait = {
            let mut st = self.lock();
            if st.next == seq {
                None
            } else {
                let (tx, rx) = oneshot::channel();
                st.waiting.insert(seq, tx);
                Some(rx)
            }
        };
        if let Some(rx) = wait {
            let _ = rx.await;
        }
        turn
    }

    fn release(&self, seq: u64) {
        let mut st = self.lock();
        st.waiting.remove(&seq);
        if seq < st.next {
            return;
        }
        st.released.insert(seq);
        while st.released.first() == Some(&st.next) {
            let n = st.next;
            st.released.remove(&n);
            st.next += 1;
        }
        let next = st.next;
        if let Some(tx) = st.waiting.remove(&next) {
            let _ = tx.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn turns_follow_ticket_order_even_when_calls_start_late() {
        rt().block_on(async {
            let seq = Sequencer::new();
            let tickets: Vec<Ticket> = (0..8).map(|_| seq.ticket()).collect();
            let log = Arc::new(Mutex::new(Vec::new()));
            let mut handles = Vec::new();
            // Start the calls in reverse order, the later ones first.
            for (i, t) in tickets.into_iter().enumerate().rev() {
                let (seq, log) = (seq.clone(), log.clone());
                handles.push(tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis((8 - i as u64) * 3)).await;
                    let _turn = seq.turn(Some(t)).await;
                    log.lock().unwrap().push(i);
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }));
            }
            for h in handles {
                h.await.unwrap();
            }
            assert_eq!(*log.lock().unwrap(), (0..8).collect::<Vec<_>>());
        });
    }

    #[test]
    fn dropped_tickets_and_cancelled_waits_give_their_place_up() {
        rt().block_on(async {
            let seq = Sequencer::new();
            let a = seq.ticket();
            let b = seq.ticket();
            let c = seq.ticket();
            let d = seq.ticket();
            drop(b); // never used
            let turn_a = seq.turn(Some(a)).await;
            // c waits for a; cancel it while it waits.
            let waiting_c = tokio::spawn({
                let seq = seq.clone();
                async move { seq.turn(Some(c)).await }
            });
            tokio::time::sleep(Duration::from_millis(10)).await;
            waiting_c.abort();
            let _ = waiting_c.await;
            drop(turn_a);
            // d gets its turn although b was dropped and c was cancelled.
            let got = tokio::time::timeout(Duration::from_secs(2), seq.turn(Some(d))).await;
            assert!(got.is_ok(), "d must not wait for b or c");
        });
    }

    #[test]
    fn a_ticket_used_through_a_clone_and_no_ticket_take_new_places() {
        rt().block_on(async {
            let seq = Sequencer::new();
            let a = seq.ticket();
            let a2 = a.clone();
            let turn = seq.turn(Some(a)).await;
            // The clone's place is used; it waits like a newcomer (after `turn`).
            let later = tokio::spawn({
                let seq = seq.clone();
                async move {
                    let _t = seq.turn(Some(a2)).await;
                }
            });
            tokio::time::sleep(Duration::from_millis(10)).await;
            assert!(!later.is_finished());
            drop(turn);
            tokio::time::timeout(Duration::from_secs(2), later)
                .await
                .unwrap()
                .unwrap();
            let _ = seq.turn(None).await;
        });
    }
}
