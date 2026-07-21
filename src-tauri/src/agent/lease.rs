//! Adaptive lease duration for claimed background tasks.
//!
//! Three buckets based on the **prompt size in characters** of the call
//! * `chars <= 8000`   →  600s
//! * `chars <= 20000`  →  900s
//! * `chars >  20000`  → 1800s

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bucket {
    Small,
    Medium,
    Large,
}

pub fn bucket_for(chars: usize) -> Bucket {
    match chars {
        0..=8000 => Bucket::Small,
        8001..=20000 => Bucket::Medium,
        _ => Bucket::Large,
    }
}

pub fn lease_seconds(chars: usize) -> u32 {
    match bucket_for(chars) {
        Bucket::Small => 600,
        Bucket::Medium => 900,
        Bucket::Large => 1800,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_bucket_is_600() {
        assert_eq!(lease_seconds(0), 600);
        assert_eq!(lease_seconds(8000), 600);
    }

    #[test]
    fn medium_bucket_is_900() {
        assert_eq!(lease_seconds(8001), 900);
        assert_eq!(lease_seconds(20_000), 900);
    }

    #[test]
    fn large_bucket_is_1800() {
        assert_eq!(lease_seconds(20_001), 1800);
        assert_eq!(lease_seconds(1_000_000), 1800);
    }
}
