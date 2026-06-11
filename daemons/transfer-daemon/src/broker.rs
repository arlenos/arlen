//! The cross-uid byte-moving seam - DEFERRED (profile-system-plan.md PR-R4, needs PR-R1 + two live profile uids).
//!
//! Moving bytes between two profile namespaces is the one thing that needs
//! simultaneous access to two profile uids, and it needs PR-R1's per-uid socket
//! move landed plus two real profile uids to exercise. So the live broker is
//! NOT built here. This module models the seam:
//!
//! - [`ApprovedTransfer`] is a capability token only [`crate::gate`] can mint
//!   (its fields are private and its constructor is crate-private). The broker's
//!   [`TransferBroker::deliver`] takes only an `ApprovedTransfer`, so the
//!   deferred live broker can never be handed an ungated transfer - the gate is
//!   the sole path to a delivery. This is the `ExecutedWrite`-opacity discipline
//!   from the agent executor.
//! - [`DeniedBroker`] is the fail-closed stand-in (the `arlen-run`
//!   `DenyUnlessEmpty` precedent): it errs on every delivery, so until the live
//!   cross-uid impl lands no transfer can move a byte. A daemon wired with it
//!   audits the gate decision and then refuses delivery, never silently
//!   succeeding.
//!
//! The live impl (open the destination profile's runtime dir, write the bytes
//! under the destination uid, single-use-clear the source clipboard handle)
//! slots in behind this trait when PR-R1 + two profile uids exist.

use async_trait::async_trait;

use crate::request::TransferRequest;

/// A transfer the gate has approved. The only mintable proof that
/// [`crate::gate::decide_transfer`] permitted a flow; the broker delivers only
/// these. Fields are private and the constructor is crate-private, so an
/// ungated transfer is unrepresentable at the delivery boundary.
#[derive(Debug, Clone)]
pub struct ApprovedTransfer {
    request: TransferRequest,
}

impl ApprovedTransfer {
    /// Mint an approval. Crate-private so only the gate, after a successful
    /// decision and dual-ledger audit, can construct one.
    pub(crate) fn new(request: TransferRequest) -> Self {
        Self { request }
    }

    /// The approved request the broker is to deliver.
    pub fn request(&self) -> &TransferRequest {
        &self.request
    }
}

/// A receipt from a completed delivery. The live broker fills it with what it
/// moved; the CORE only names the type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryReceipt {
    /// Bytes moved across the boundary, when measured.
    pub bytes: Option<u64>,
}

/// Why a cross-profile delivery could not be performed.
#[derive(Debug, thiserror::Error)]
pub enum BrokerError {
    /// No live cross-uid broker is wired, so the transfer cannot be performed.
    /// Refuse rather than silently drop the bytes.
    #[error("the cross-profile broker is not yet wired; refusing the delivery")]
    NoBroker,
}

/// Moves an approved transfer's bytes from the source profile namespace into the
/// destination profile namespace. The single privileged dual-uid operation; the
/// trait keeps the gate decoupled from the on-system cross-uid machinery.
#[async_trait]
pub trait TransferBroker: Send + Sync {
    /// Deliver the approved transfer. Only an [`ApprovedTransfer`] (gate-minted)
    /// can be passed, so an ungated transfer never reaches a delivery.
    async fn deliver(&self, approved: &ApprovedTransfer)
        -> Result<DeliveryReceipt, BrokerError>;
}

/// The fail-closed stand-in used until the live cross-uid broker is wired. Every
/// delivery is refused, so no transfer moves a byte; the daemon still audits the
/// gate decision first.
pub struct DeniedBroker;

#[async_trait]
impl TransferBroker for DeniedBroker {
    async fn deliver(
        &self,
        _approved: &ApprovedTransfer,
    ) -> Result<DeliveryReceipt, BrokerError> {
        Err(BrokerError::NoBroker)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{PayloadRef, ProfileId, TransferType};

    fn approved() -> ApprovedTransfer {
        ApprovedTransfer::new(TransferRequest {
            source: ProfileId::new("work").unwrap(),
            dest: ProfileId::new("personal").unwrap(),
            ty: TransferType::File,
            payload: PayloadRef::File {
                source_path: "/home/work/report.pdf".into(),
            },
        })
    }

    #[tokio::test]
    async fn the_denied_broker_refuses_every_delivery() {
        // Until the live cross-uid impl lands, no transfer moves a byte.
        let broker = DeniedBroker;
        let err = broker
            .deliver(&approved())
            .await
            .expect_err("the stand-in refuses");
        assert!(matches!(err, BrokerError::NoBroker));
    }

    #[test]
    fn an_approval_carries_its_request() {
        let a = approved();
        assert_eq!(a.request().source.as_str(), "work");
        assert_eq!(a.request().dest.as_str(), "personal");
    }
}
