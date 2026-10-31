//! The per-frame gate that skips instance uploads whose bytes are already in
//! the GPU buffer.

use bytemuck::Pod;

/// Decides whether a frame's instance data must be written to its GPU buffer.
///
/// Cupid re-records the draw list every frame, so each pipeline rebuilds its
/// instance vector even when nothing on screen moved. Writing that vector to
/// the GPU anyway costs a staging-buffer allocation and a blit per
/// [`wgpu::Queue::write_buffer`] — on a mostly static frame these redundant
/// uploads dominate the CPU encode time. `FrameUpload` remembers the bytes
/// that were last written to the buffer, letting the pipeline skip the write
/// when the new frame's bytes are already there.
///
/// # Contract
///
/// One `FrameUpload` guards exactly one buffer, and every guarded write must
/// start at offset zero — which is how Cupid's pipelines upload since the
/// whole frame's instances travel in a single write. Whenever the buffer is
/// recreated (grown or shrunk by its size policy), call [`invalidate`] so the
/// next frame writes unconditionally.
///
/// A skipped upload relies on the destination buffer retaining its contents
/// between frames, which `wgpu` guarantees for a buffer that is not
/// reallocated.
///
/// # Examples
///
/// ```ignore
/// let mut gate = FrameUpload::new();
/// // Frame 1: fresh gate, the bytes must go up.
/// assert!(gate.needs_upload(&instances));
/// gate.mark_uploaded(&instances);
/// // Frame 2: same scene, same bytes — nothing to do.
/// assert!(!gate.needs_upload(&instances));
/// ```
///
/// [`invalidate`]: FrameUpload::invalidate
pub(crate) struct FrameUpload<T> {
    /// The bytes the GPU buffer holds, starting at offset zero.
    uploaded: Vec<T>,
    /// Raised while the buffer's contents are unknown: before the first
    /// upload, and after every reallocation.
    buffer_lost: bool,
}

impl<T: Pod> FrameUpload<T> {
    /// Creates a gate for a buffer whose contents are still undefined, so the
    /// first frame always uploads.
    pub(crate) fn new() -> Self {
        Self {
            uploaded: Vec::new(),
            buffer_lost: true,
        }
    }

    /// Marks the buffer's contents undefined.
    ///
    /// Call this whenever the guarded buffer is recreated — a fresh buffer
    /// holds garbage no matter how familiar the frame's instances look.
    pub(crate) fn invalidate(&mut self) {
        self.uploaded.clear();
        self.buffer_lost = true;
    }

    /// Whether `current` must be written before this frame's draws execute.
    ///
    /// `false` means every byte a draw will read is already in the buffer:
    /// either this frame equals the last upload exactly, or it is a prefix of
    /// it — the tail beyond `current.len()` is never referenced by a draw, so
    /// it cannot matter.
    pub(crate) fn needs_upload(&self, current: &[T]) -> bool {
        if self.buffer_lost {
            return true;
        }
        let Some(head) = self.uploaded.get(..current.len()) else {
            return true;
        };
        bytemuck::cast_slice::<T, u8>(head) != bytemuck::cast_slice::<T, u8>(current)
    }

    /// Records that `current` was just written to the buffer at offset zero.
    pub(crate) fn mark_uploaded(&mut self, current: &[T]) {
        self.uploaded.clear();
        self.uploaded.extend_from_slice(current);
        self.buffer_lost = false;
    }
}

impl<T: Pod> FrameUpload<T> {
    /// Backend-agnostic upload that writes `current` into `buffer` via
    /// the given backend. Skips the write when the buffer already holds
    /// the same bytes.
    pub(crate) fn upload<B: crate::backend::GpuBackend>(
        &mut self,
        backend: &B,
        buffer: &B::Buffer,
        current: &[T],
    ) -> bool {
        if current.is_empty() || !self.needs_upload(current) {
            return false;
        }
        backend.write_buffer(buffer, 0, bytemuck::cast_slice(current));
        self.mark_uploaded(current);
        true
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use aimer_time::Instant;

    use super::*;

    #[test]
    fn a_fresh_gate_requires_the_first_upload() {
        let gate = FrameUpload::<u32>::new();

        assert!(gate.needs_upload(&[1, 2, 3]));
    }

    #[test]
    fn an_unchanged_frame_skips_the_upload() {
        let mut gate = FrameUpload::new();
        gate.mark_uploaded(&[1, 2, 3]);

        assert!(!gate.needs_upload(&[1, 2, 3]));
    }

    // A shorter frame whose bytes match the head of the last upload reads
    // only bytes that are already in the buffer — the stale tail beyond it is
    // never referenced by a draw.
    #[test]
    fn a_prefix_of_the_last_upload_skips() {
        let mut gate = FrameUpload::new();
        gate.mark_uploaded(&[1, 2, 3]);

        assert!(!gate.needs_upload(&[1, 2]));
        assert!(!gate.needs_upload(&[]));
    }

    #[test]
    fn changed_bytes_force_an_upload() {
        let mut gate = FrameUpload::new();
        gate.mark_uploaded(&[1, 2, 3]);

        assert!(gate.needs_upload(&[1, 9, 3]));
        assert!(gate.needs_upload(&[2, 3]));
    }

    #[test]
    fn a_longer_frame_forces_an_upload() {
        let mut gate = FrameUpload::new();
        gate.mark_uploaded(&[1, 2, 3]);

        assert!(gate.needs_upload(&[1, 2, 3, 4]));
    }

    // A recreated buffer holds garbage, so even byte-identical instances must
    // be written again.
    #[test]
    fn a_recreated_buffer_forces_an_upload() {
        let mut gate = FrameUpload::new();
        gate.mark_uploaded(&[1, 2, 3]);
        gate.invalidate();

        assert!(gate.needs_upload(&[1, 2, 3]));
    }

    // Skipping must not forget what the buffer really holds: after a shorter
    // frame rode on the previous upload, a frame equal to the original upload
    // still finds all of its bytes in place.
    #[test]
    fn skipping_does_not_forget_what_the_buffer_holds() {
        let mut gate = FrameUpload::new();
        gate.mark_uploaded(&[1, 2, 3]);

        assert!(!gate.needs_upload(&[1, 2]));
        assert!(!gate.needs_upload(&[1, 2, 3]));
    }

    #[test]
    fn recording_a_new_upload_replaces_the_remembered_bytes() {
        let mut gate = FrameUpload::new();
        gate.mark_uploaded(&[1, 2, 3]);
        gate.mark_uploaded(&[7, 8]);

        assert!(!gate.needs_upload(&[7, 8]));
        assert!(gate.needs_upload(&[1, 2, 3]));
    }

    #[test]
    #[ignore = "manual bulk-data profile"]
    fn profile_bulk_upload_operations() {
        const ROUNDS: usize = 7;

        let cases = [
            ("empty", 0, 1_024),
            ("small-64b", 16, 1_024),
            ("medium-4kb", 1_024, 128),
            ("large-256kb", 65_536, 16),
            ("large-4mb", 1_048_576, 2),
        ];
        let mut checksum = 0u64;

        for (name, word_count, measured) in cases {
            let data: Vec<u32> = (0..word_count)
                .map(|index| (index as u32).wrapping_mul(0x9e37_79b9))
                .collect();

            let mut compare_samples = Vec::with_capacity(ROUNDS);
            let mut gate = FrameUpload::new();
            gate.mark_uploaded(&data);
            for _ in 0..ROUNDS {
                for _ in 0..2 {
                    checksum = checksum.wrapping_add(black_box(gate.needs_upload(&data)) as u64);
                }

                let start = Instant::now();
                for _ in 0..measured {
                    checksum = checksum.wrapping_add(black_box(gate.needs_upload(&data)) as u64);
                }
                compare_samples.push(start.elapsed().as_secs_f64() * 1e6 / measured as f64);
            }

            let mut copy_samples = Vec::with_capacity(ROUNDS);
            for _ in 0..ROUNDS {
                for _ in 0..2 {
                    gate.mark_uploaded(&data);
                    black_box(gate.uploaded.as_ptr());
                }

                let start = Instant::now();
                for _ in 0..measured {
                    gate.mark_uploaded(&data);
                    black_box(gate.uploaded.as_ptr());
                }
                copy_samples.push(start.elapsed().as_secs_f64() * 1e6 / measured as f64);
            }

            compare_samples.sort_by(f64::total_cmp);
            copy_samples.sort_by(f64::total_cmp);
            let compare_p50 = compare_samples[ROUNDS / 2];
            let compare_p95 = compare_samples[(ROUNDS * 95).div_ceil(100) - 1];
            let copy_p50 = copy_samples[ROUNDS / 2];
            let copy_p95 = copy_samples[(ROUNDS * 95).div_ceil(100) - 1];
            println!(
                "{name}: compare p50 {compare_p50:.3} us, p95 {compare_p95:.3} us; copy p50 {copy_p50:.3} us, p95 {copy_p95:.3} us"
            );
        }

        black_box(&mut checksum);
    }
}
