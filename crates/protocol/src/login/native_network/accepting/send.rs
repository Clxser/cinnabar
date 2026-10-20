//! Application packet projection into the original accepted connection's prepared effect.

use super::NativeAcceptingIo;
use bedrock_client_tokio::connection::send::{PreparedSend, SendRetirement};
use bedrock_protocol_1_26_51::packets;
use bytes::Bytes;
use std::{io, time::Instant};

impl NativeAcceptingIo {
    /// Encodes typed effects once without changing the cipher or admitting a command.
    /// Rejects another retained effect or a locally retired send direction.
    pub fn prepare_packets(
        &mut self,
        packets: Vec<packets::Packet>,
        deadline: Instant,
    ) -> io::Result<PreparedSend<'_>> {
        let packets: Vec<_> = packets.into_iter().map(crate::Packet::new).collect();
        let body = if packets.is_empty() {
            Bytes::new()
        } else {
            crate::session_transport::batch_body(&packets).ok_or_else(super::invalid)?
        };
        self.connection.prepare_body(body, deadline)
    }

    /// Encodes one original complete raw packet without changing its bytes or cipher.
    pub fn prepare_raw(
        &mut self,
        packet: Bytes,
        deadline: Instant,
    ) -> io::Result<PreparedSend<'_>> {
        let body = self
            .connection
            .raw_body(packet)
            .map_err(|_| super::invalid())?;
        self.connection.prepare_body(body, deadline)
    }

    /// Resumes the exact cursor, permit or receipt retained by an earlier borrowed operation.
    pub fn resume_send(&mut self) -> io::Result<PreparedSend<'_>> {
        self.connection.resume_send()
    }

    /// Seals new local commits and caps settlement by the first cleanup and original send cutoffs.
    /// This does not close the association. Settle its receipt before calling begin_close.
    pub fn begin_send_retirement(&mut self, deadline: Instant) -> SendRetirement<'_> {
        self.connection.begin_send_retirement(deadline)
    }
}

#[cfg(test)]
pub(in crate::login::native_network) mod tests;
