use std::{
    io::ErrorKind,
    net::{SocketAddr, UdpSocket},
};

use crate::{NetworkError, MAX_NETWORK_PACKET_BYTES};

pub type ReceivedPacket<A> = (A, Vec<u8>);

pub trait NetworkTransport: Send {
    type Address: Clone + Ord + std::fmt::Debug + Send + 'static;

    fn send(&mut self, destination: &Self::Address, payload: &[u8]) -> Result<(), NetworkError>;
    fn receive(&mut self) -> Result<Option<ReceivedPacket<Self::Address>>, NetworkError>;
    fn local_address(&self) -> Result<Self::Address, NetworkError>;
}

/// Non-blocking UDP transport suitable for authoritative or peer-hosted
/// sessions. `NetworkSession` adds a bounded replay window plus acknowledgement
/// and retransmission for reliable control/state/RPC messages.
pub struct UdpTransport {
    socket: UdpSocket,
}

impl UdpTransport {
    pub fn bind(address: SocketAddr) -> Result<Self, NetworkError> {
        let socket =
            UdpSocket::bind(address).map_err(|error| NetworkError::Transport(error.to_string()))?;
        socket
            .set_nonblocking(true)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        Ok(Self { socket })
    }
}

impl NetworkTransport for UdpTransport {
    type Address = SocketAddr;

    fn send(&mut self, destination: &Self::Address, payload: &[u8]) -> Result<(), NetworkError> {
        if payload.len() > MAX_NETWORK_PACKET_BYTES {
            return Err(NetworkError::PacketTooLarge);
        }
        let sent = self
            .socket
            .send_to(payload, destination)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        if sent != payload.len() {
            return Err(NetworkError::Transport("partial UDP datagram send".into()));
        }
        Ok(())
    }

    fn receive(&mut self) -> Result<Option<ReceivedPacket<Self::Address>>, NetworkError> {
        let mut payload = vec![0_u8; MAX_NETWORK_PACKET_BYTES];
        match self.socket.recv_from(&mut payload) {
            Ok((length, source)) => {
                payload.truncate(length);
                Ok(Some((source, payload)))
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(NetworkError::Transport(error.to_string())),
        }
    }

    fn local_address(&self) -> Result<Self::Address, NetworkError> {
        self.socket
            .local_addr()
            .map_err(|error| NetworkError::Transport(error.to_string()))
    }
}
