use super::super::model::CullRecord;
use super::*;

impl DirectOcclusion {
    /// Reads the GPU records used to compute this publication's direct-draw occlusion.
    pub(in crate::chunk::gpu_cull) fn publication_snapshot(
        &self,
        device: &RenderDevice,
        queue: &RenderQueue,
    ) -> Vec<Vec<u8>> {
        self.storage.as_ref().map_or_else(Vec::new, |storage| {
            vec![crate::render_test_support::read_buffer(
                device,
                queue,
                &storage.records,
                0,
                u64::from(self.table.slot_count()) * std::mem::size_of::<CullRecord>() as u64,
            )]
        })
    }
}
