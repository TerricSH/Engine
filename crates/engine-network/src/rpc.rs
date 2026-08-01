use std::collections::{BTreeMap, VecDeque};

use crate::{NetworkError, PeerId, RpcEnvelope, RpcTarget, MAX_RPC_PAYLOAD_BYTES};

pub type RpcHandler = Box<dyn FnMut(&RpcEnvelope) -> Result<(), String> + Send>;

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

    pub fn dispatch(&mut self, limit: usize) -> Vec<(u64, Result<(), String>)> {
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
