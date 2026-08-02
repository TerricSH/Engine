use std::collections::{BTreeMap, VecDeque};

use crate::{NetworkError, PeerId, RpcEnvelope, RpcTarget, MAX_RPC_PAYLOAD_BYTES};

pub type RpcHandler = Box<dyn FnMut(&RpcEnvelope) -> Result<(), String> + Send>;
pub type RpcDispatchResults = Vec<(u64, Result<(), String>)>;

#[derive(Default)]
pub struct RpcRouter {
    handlers: BTreeMap<String, RpcHandler>,
    incoming: VecDeque<RpcEnvelope>,
    next_rpc_id: u64,
}

impl RpcRouter {
    pub fn register(
        &mut self,
        method: impl Into<String>,
        handler: RpcHandler,
    ) -> Result<(), NetworkError> {
        let method = method.into();
        validate_method(&method)?;
        self.handlers.insert(method, handler);
        Ok(())
    }

    pub fn envelope(
        &mut self,
        sender: PeerId,
        target: RpcTarget,
        method: impl Into<String>,
        reliable: bool,
        payload: Vec<u8>,
    ) -> Result<RpcEnvelope, NetworkError> {
        let method = method.into();
        validate_method(&method)?;
        if payload.len() > MAX_RPC_PAYLOAD_BYTES {
            return Err(NetworkError::LimitExceeded("RPC payload"));
        }
        self.next_rpc_id = self.next_rpc_id.saturating_add(1);
        Ok(RpcEnvelope {
            rpc_id: self.next_rpc_id,
            sender,
            target,
            method,
            reliable,
            payload,
        })
    }

    pub fn enqueue(&mut self, envelope: RpcEnvelope) -> Result<(), NetworkError> {
        validate_method(&envelope.method)?;
        if envelope.payload.len() > MAX_RPC_PAYLOAD_BYTES {
            return Err(NetworkError::LimitExceeded("RPC payload"));
        }
        if self.incoming.len() >= 4096 {
            return Err(NetworkError::LimitExceeded("RPC queue"));
        }
        self.incoming.push_back(envelope);
        Ok(())
    }

    pub fn dispatch(&mut self, limit: usize) -> RpcDispatchResults {
        let mut results = Vec::new();
        for _ in 0..limit.max(1) {
            let Some(envelope) = self.incoming.pop_front() else {
                break;
            };
            let result = self.handlers.get_mut(&envelope.method).map_or_else(
                || Err(format!("unregistered RPC method '{}'", envelope.method)),
                |handler| handler(&envelope),
            );
            results.push((envelope.rpc_id, result));
        }
        results
    }

    /// Dispatch native handlers and return unregistered envelopes intact so
    /// the gameplay bridge can deliver them as frame-local script events.
    pub fn dispatch_registered(&mut self, limit: usize) -> (RpcDispatchResults, Vec<RpcEnvelope>) {
        let mut results = Vec::new();
        let mut unhandled = Vec::new();
        for _ in 0..limit.max(1) {
            let Some(envelope) = self.incoming.pop_front() else {
                break;
            };
            if let Some(handler) = self.handlers.get_mut(&envelope.method) {
                results.push((envelope.rpc_id, handler(&envelope)));
            } else {
                unhandled.push(envelope);
            }
        }
        (results, unhandled)
    }
}

fn validate_method(method: &str) -> Result<(), NetworkError> {
    if method.is_empty()
        || method.len() > 128
        || !method
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(NetworkError::LimitExceeded("RPC method name"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_handlers_run_while_unhandled_envelopes_remain_for_scripts() {
        let mut router = RpcRouter::default();
        router
            .register("native.call", Box::new(|_| Ok(())))
            .unwrap();
        let native = router
            .envelope(PeerId(2), RpcTarget::Server, "native.call", true, vec![])
            .unwrap();
        let scripted = router
            .envelope(PeerId(3), RpcTarget::All, "game.event", false, vec![7])
            .unwrap();
        router.enqueue(native).unwrap();
        router.enqueue(scripted.clone()).unwrap();

        let (results, unhandled) = router.dispatch_registered(8);
        assert_eq!(results.len(), 1);
        assert!(results[0].1.is_ok());
        assert_eq!(unhandled, vec![scripted]);
    }

    #[test]
    fn legacy_dispatch_preserves_mixed_envelope_order() {
        let mut router = RpcRouter::default();
        router
            .register("native.call", Box::new(|_| Ok(())))
            .unwrap();
        let missing = router
            .envelope(PeerId(3), RpcTarget::All, "game.event", false, vec![])
            .unwrap();
        let native = router
            .envelope(PeerId(2), RpcTarget::Server, "native.call", true, vec![])
            .unwrap();
        router.enqueue(missing.clone()).unwrap();
        router.enqueue(native.clone()).unwrap();

        let results = router.dispatch(8);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, missing.rpc_id);
        assert!(results[0].1.is_err());
        assert_eq!(results[1].0, native.rpc_id);
        assert!(results[1].1.is_ok());
    }
}
