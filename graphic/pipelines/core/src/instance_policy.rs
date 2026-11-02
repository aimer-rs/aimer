/// Sizes a per-frame instance buffer: grows at once, shrinks after sustained low use.
#[derive(Debug)]
pub struct InstanceBufferPolicy {
    initial_capacity: usize,
    capacity: usize,
    underused_frames: u16,
}

impl InstanceBufferPolicy {
    /// Consecutive underused frames before capacity is reclaimed.
    pub const SHRINK_AFTER_FRAMES: u16 = 120;

    /// Creates a policy holding `initial_capacity` instances.
    pub const fn new(initial_capacity: usize) -> Self {
        Self {
            initial_capacity,
            capacity: initial_capacity,
            underused_frames: 0,
        }
    }

    /// Returns the current capacity in instances.
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Records one frame's instance count, possibly growing or shrinking capacity.
    pub fn record_usage(&mut self, used: usize) {
        let required = self.initial_capacity.max(used.next_power_of_two());
        if required > self.capacity {
            self.capacity = required;
            self.underused_frames = 0;
        } else if required <= self.capacity / 4 {
            self.underused_frames = self.underused_frames.saturating_add(1);
            if self.underused_frames >= Self::SHRINK_AFTER_FRAMES {
                self.capacity = required;
                self.underused_frames = 0;
            }
        } else {
            self.underused_frames = 0;
        }
    }

    /// Grows capacity immediately to hold `required` instances.
    pub fn grow_to_fit(&mut self, required: usize) {
        if required > self.capacity {
            self.capacity = required.next_power_of_two();
            self.underused_frames = 0;
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sustained_low_usage_reclaims_peak_instance_capacity() {
        let mut policy = InstanceBufferPolicy::new(64);
        policy.record_usage(4096);
        assert_eq!(policy.capacity(), 4096);

        for _ in 0..InstanceBufferPolicy::SHRINK_AFTER_FRAMES {
            policy.record_usage(32);
        }

        assert_eq!(policy.capacity(), 64);
    }
}
