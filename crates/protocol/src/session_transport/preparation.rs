//! Requested protocol controls remain live while presentation preparation holds play packets.

#[cfg(all(feature = "native-networking", not(target_arch = "wasm32")))]
use super::Reader;
use super::{ReadItem, SessionTransport, ends_session, pop_read_ahead};
use crate::raw_packet::RawPacket;
use crate::{Packet, ProtocolError, SessionError};
use bedrock_protocol_1_26_51::packets::{self, PacketId};
use std::future::poll_fn;

impl<R: bridge::FrameRead> SessionTransport<R> {
    /// Retains ordinary packets on the existing FIFO while answering eligible latency requests.
    pub(crate) async fn read_ahead_during_preparation(
        &mut self,
        max_bytes: usize,
    ) -> Result<(), ProtocolError> {
        // An earlier canceled wait may already own an admitted response.
        self.drain_send().await?;
        loop {
            let available = max_bytes.saturating_sub(self.preparation_bytes);
            #[cfg(all(feature = "native-networking", not(target_arch = "wasm32")))]
            if matches!(&self.reader, Reader::Native(_)) {
                self.apply_native_read_ahead(available);
            }
            self.split_preparation_batches()?;
            if self.buffered_startup_terminal()?.is_some() {
                return Ok(());
            }
            if let Some(reply) = self.take_preparation_reply() {
                // The first send poll retains its owner before the next suspension.
                self.send(&[reply]).await?;
                continue;
            }
            #[cfg(all(feature = "native-networking", not(target_arch = "wasm32")))]
            if let Some(front) = self.ahead.front()
                && let Some(Ok(ReadItem::Native(batch))) = &front.item
                && !batch.relay_applied
            {
                // No later callback or control may pass this original delayed front.
                if let Some(deadline) = front.delay
                    && let Some(delay) = self.packet_delay()
                {
                    delay.wait(Some(deadline)).await;
                    continue;
                }
                return Ok(());
            }
            let available = max_bytes.saturating_sub(self.preparation_bytes);
            if self.ahead_bytes >= available
                || self
                    .ahead
                    .back()
                    .is_some_and(|entry| ends_session(&entry.item))
            {
                return Ok(());
            }
            poll_fn(|cx| self.poll_read_ahead_one(available, cx)).await;
        }
    }

    /// Releases the preparation charge only after its actual finite job has completed.
    pub(crate) fn complete_preparation(&mut self) {
        self.preparation_bytes = 0;
    }

    /// Splits only eligible original batches through the normal relay and tap owners.
    fn split_preparation_batches(&mut self) -> Result<(), SessionError> {
        if self.buffered_startup_terminal()?.is_some() {
            return Ok(());
        }
        while let Some(batch) = self.startup.pop_front() {
            self.receive_retained_batch(batch, 0)?;
            if self.buffered_startup_terminal()?.is_some() {
                return Ok(());
            }
        }
        loop {
            let eligible = match self.ahead.front().and_then(|entry| entry.item.as_ref()) {
                Some(Ok(ReadItem::Framed(frame))) => {
                    crate::session_wire::batch_frame_body(frame).is_some()
                }
                #[cfg(all(feature = "native-networking", not(target_arch = "wasm32")))]
                Some(Ok(ReadItem::Native(batch))) => batch.relay_applied,
                _ => false,
            };
            if !eligible {
                return Ok(());
            }
            let entry = pop_read_ahead(&mut self.ahead, &mut self.ahead_bytes)
                .expect("the eligible original batch is retained");
            let previous = self.received.len();
            self.split_frames += 1;
            match entry.item.expect("an eligible batch exists")? {
                ReadItem::Framed(frame) => {
                    let body = crate::session_wire::batch_frame_body(&frame)
                        .expect("the eligible frame is a batch");
                    self.receive_server_batch(body, self.split_frames)?;
                }
                #[cfg(all(feature = "native-networking", not(target_arch = "wasm32")))]
                ReadItem::Native(batch) => {
                    self.receive_retained_batch(batch, self.split_frames)?;
                }
            }
            let retained = self
                .received
                .range(previous..)
                .map(RawPacket::backing_len)
                .max()
                .unwrap_or(0);
            self.preparation_bytes = self
                .preparation_bytes
                .saturating_add(entry.charge.max(retained));
            if self.buffered_startup_terminal()?.is_some() {
                return Ok(());
            }
        }
    }

    /// Removes one requested probe; malformed and unrequested probes keep their ordinary handling.
    fn take_preparation_reply(&mut self) -> Option<Packet> {
        let (index, timestamp) = self.received.iter().enumerate().find_map(|(index, raw)| {
            if raw.id != PacketId::NetworkStackLatency {
                return None;
            }
            let packets::Packet::NetworkStackLatency(probe) = *raw.clone().decode().ok()?.data
            else {
                return None;
            };
            probe
                .is_from_server
                .then_some((index, probe.creation_time.0))
        })?;
        self.received.remove(index);
        Some(crate::network_stack_latency_reply(timestamp))
    }
}
