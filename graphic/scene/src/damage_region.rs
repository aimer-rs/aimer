//! Device-pixel damage tracking used by the persistent scene target.

use smallvec::SmallVec;

/// Maximum number of disjoint regions carried by one partial frame.
pub const MAX_DAMAGE_REGIONS: usize = 8;

/// A half-open rectangle in physical device pixels.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DamageRect {
    /// Left edge in target pixels.
    pub x: u32,
    /// Top edge in target pixels.
    pub y: u32,
    /// Width in target pixels.
    pub width: u32,
    /// Height in target pixels.
    pub height: u32,
}

impl DamageRect {
    #[inline]
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    #[inline]
    fn right(self) -> u64 {
        u64::from(self.x) + u64::from(self.width)
    }

    #[inline]
    fn bottom(self) -> u64 {
        u64::from(self.y) + u64::from(self.height)
    }

    #[inline]
    fn area(self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    #[inline]
    fn clipped(self, target_width: u32, target_height: u32) -> Option<Self> {
        if self.width == 0 || self.height == 0 || target_width == 0 || target_height == 0 {
            return None;
        }
        let x = self.x.min(target_width);
        let y = self.y.min(target_height);
        let right = self.right().min(u64::from(target_width));
        let bottom = self.bottom().min(u64::from(target_height));
        (right > u64::from(x) && bottom > u64::from(y)).then_some(Self::new(
            x,
            y,
            (right - u64::from(x)) as u32,
            (bottom - u64::from(y)) as u32,
        ))
    }
}

/// Normalized damage for one target.
///
/// Regions are clipped to the target, merged when they overlap or touch, and
/// promoted to a full repaint when a partial submission would no longer be
/// cheaper or when the caller reports unknown damage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DamageSet {
    target_width: u32,
    target_height: u32,
    full: bool,
    regions: SmallVec<[DamageRect; MAX_DAMAGE_REGIONS]>,
}

impl DamageSet {
    #[inline]
    pub fn new(target_width: u32, target_height: u32) -> Self {
        Self {
            target_width,
            target_height,
            full: false,
            regions: SmallVec::new(),
        }
    }

    #[inline]
    pub fn full(target_width: u32, target_height: u32) -> Self {
        let mut damage = Self::new(target_width, target_height);
        damage.mark_full();
        damage
    }

    /// Adds a device-pixel rectangle to the set.
    pub fn add(&mut self, rectangle: DamageRect) {
        if self.full {
            return;
        }
        let Some(mut merged) = rectangle.clipped(self.target_width, self.target_height) else {
            return;
        };

        loop {
            let mut changed = false;
            let mut index = 0;
            while index < self.regions.len() {
                if touches_or_overlaps(merged, self.regions[index]) {
                    merged = union(merged, self.regions.remove(index));
                    changed = true;
                } else {
                    index += 1;
                }
            }
            if !changed {
                break;
            }
        }

        self.regions.push(merged);
        self.regions
            .sort_unstable_by_key(|region| (region.y, region.x));

        if self.regions.len() > MAX_DAMAGE_REGIONS
            || self.covered_area() >= half_area(self.target_width, self.target_height)
        {
            self.mark_full();
        }
    }

    /// Promotes this set to a full target repaint.
    #[inline]
    pub fn mark_full(&mut self) {
        self.full = true;
        self.regions.clear();
        if self.target_width != 0 && self.target_height != 0 {
            self.regions
                .push(DamageRect::new(0, 0, self.target_width, self.target_height));
        }
    }

    #[inline]
    pub const fn is_full(&self) -> bool {
        self.full
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        !self.full && self.regions.is_empty()
    }

    #[inline]
    pub fn regions(&self) -> &[DamageRect] {
        &self.regions
    }

    #[inline]
    pub const fn target_size(&self) -> (u32, u32) {
        (self.target_width, self.target_height)
    }

    fn covered_area(&self) -> u64 {
        self.regions
            .iter()
            .map(|region| region.area())
            .fold(0, u64::saturating_add)
    }
}

#[inline]
fn touches_or_overlaps(left: DamageRect, right: DamageRect) -> bool {
    left.x as u64 <= right.right()
        && right.x as u64 <= left.right()
        && left.y as u64 <= right.bottom()
        && right.y as u64 <= left.bottom()
}

#[inline]
fn union(left: DamageRect, right: DamageRect) -> DamageRect {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    let right_edge = left.right().max(right.right());
    let bottom_edge = left.bottom().max(right.bottom());
    DamageRect::new(
        x,
        y,
        (right_edge - u64::from(x)) as u32,
        (bottom_edge - u64::from(y)) as u32,
    )
}

#[inline]
fn half_area(width: u32, height: u32) -> u64 {
    let area = u64::from(width) * u64::from(height);
    area / 2 + area % 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_regions_are_clipped_and_adjacent_regions_are_merged() {
        let mut damage = DamageSet::new(100, 80);
        damage.add(DamageRect::new(90, 70, 20, 20));
        damage.add(DamageRect::new(0, 0, 5, 5));
        damage.add(DamageRect::new(5, 0, 5, 5));

        assert!(!damage.is_full());
        assert_eq!(
            damage.regions(),
            &[
                DamageRect::new(0, 0, 10, 5),
                DamageRect::new(90, 70, 10, 10),
            ]
        );
    }
}
