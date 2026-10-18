//! Upstream backup (design §7.3): a message that moves responsibility for a branch is kept until
//! its receiver has logged what it produced. Only then does the sender let go of its own record.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::Inner;
use crate::ids::WorkerId;
use crate::log::{Durable, Record};
use crate::message::{Delivery, Message};

/// A message kept until its receiver acknowledges it.
pub(super) struct Outgoing {
    /// Who it was addressed to; if that worker dies, its successor gets the message.
    pub to: WorkerId,
    /// Who it was last sent to.
    pub sent_to: WorkerId,
    pub message: Message,
    pub release: Option<Arc<Release>>,
}

/// A log record of this worker's that goes once every message taking over from it is
/// acknowledged (and any other durable step counted in).
pub(super) struct Release {
    remaining: AtomicUsize,
    record: Record,
}

impl Release {
    /// Releases `record` after `parts` acknowledgements or durable writes.
    pub fn new(parts: usize, record: Record) -> Arc<Self> {
        Arc::new(Self {
            remaining: AtomicUsize::new(parts),
            record,
        })
    }
}

impl Inner {
    /// Sends a message built with its delivery, and keeps it until acknowledged.
    pub(super) fn send_kept(
        self: &Arc<Self>,
        to: WorkerId,
        build: impl FnOnce(Delivery) -> Message,
        release: Option<Arc<Release>>,
    ) {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let message = build(Delivery { from: self.id, seq });
        let target = self.routes.resolve(to);
        let kept = Outgoing {
            to,
            sent_to: target,
            message: message.clone(),
            release,
        };
        self.outbox.lock().unwrap().insert(seq, kept);
        self.deliver(target, message);
    }

    /// Acknowledges `delivery` to its sender.
    pub(super) fn ack(self: &Arc<Self>, delivery: Delivery) {
        let ack = Message::Ack {
            seqs: vec![delivery.seq],
        };
        self.deliver(delivery.from, ack);
    }

    /// Acknowledges `delivery` once `durable` resolves.
    pub(super) fn ack_when(self: &Arc<Self>, durable: Durable, delivery: Delivery) {
        let worker = self.clone();
        tokio::spawn(async move {
            if durable.await.is_ok() {
                worker.ack(delivery);
            }
        });
    }

    pub(super) fn on_ack(self: &Arc<Self>, seqs: Vec<u64>) {
        for seq in seqs {
            let kept = self.outbox.lock().unwrap().remove(&seq);
            if let Some(release) = kept.and_then(|kept| kept.release) {
                self.release(&release);
            }
        }
    }

    /// Counts one part of `release` done; the last one drops its record.
    pub(super) fn release(&self, release: &Release) {
        if release.remaining.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.log.record(vec![release.record.clone()]);
        }
    }

    /// Counts a part of `release` done once `durable` resolves.
    pub(super) fn release_when(self: &Arc<Self>, durable: Durable, release: Arc<Release>) {
        let worker = self.clone();
        tokio::spawn(async move {
            if durable.await.is_ok() {
                worker.release(&release);
            }
        });
    }

    /// Re-sends kept messages whose receiver is no longer where they were sent.
    pub(super) fn resend_moved(self: &Arc<Self>) {
        let moved: Vec<(WorkerId, Message)> = {
            let mut outbox = self.outbox.lock().unwrap();
            outbox
                .values_mut()
                .filter_map(|kept| {
                    let target = self.routes.resolve(kept.to);
                    (target != kept.sent_to).then(|| {
                        kept.sent_to = target;
                        (target, kept.message.clone())
                    })
                })
                .collect()
        };
        for (target, message) in moved {
            tracing::info!(
                target,
                "re-sending a message its receiver did not acknowledge"
            );
            self.deliver(target, message);
        }
    }
}
