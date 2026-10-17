//! Star-rating buckets and selection of difficulties by bucket.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::Serialize;

use super::osucollector::Beatmap;

/// Highest bucket; everything at or above it is shown as "10★+".
pub const MAX_BUCKET: u8 = 10;

/// Bucket N holds `N <= SR < N+1`; SR >= 10 goes to the top bucket.
pub fn bucket_of(sr: f64) -> u8 {
    if !sr.is_finite() || sr < 0.0 {
        return 0;
    }
    (sr.floor() as u64).min(MAX_BUCKET as u64) as u8
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Bucket {
    pub star: u8,
    pub diffs: u32,
    pub sets: u32,
}

pub fn histogram(beatmaps: &[Beatmap]) -> Vec<Bucket> {
    let mut diffs = [0u32; MAX_BUCKET as usize + 1];
    let mut sets: Vec<HashSet<u64>> = vec![HashSet::new(); MAX_BUCKET as usize + 1];
    for b in beatmaps {
        let k = bucket_of(b.sr) as usize;
        diffs[k] += 1;
        sets[k].insert(b.set_id);
    }
    (0..=MAX_BUCKET)
        .map(|s| Bucket {
            star: s,
            diffs: diffs[s as usize],
            sets: sets[s as usize].len() as u32,
        })
        .collect()
}

/// Selected difficulties grouped by beatmapset id.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Selection {
    pub by_set: BTreeMap<u64, BTreeSet<String>>,
}

impl Selection {
    pub fn diff_count(&self) -> usize {
        self.by_set.values().map(BTreeSet::len).sum()
    }
}

pub fn select<'a>(beatmaps: impl IntoIterator<Item = &'a Beatmap>, buckets: &[u8]) -> Selection {
    let wanted: HashSet<u8> = buckets.iter().copied().collect();
    let mut sel = Selection::default();
    for b in beatmaps {
        if wanted.contains(&bucket_of(b.sr)) {
            sel.by_set
                .entry(b.set_id)
                .or_default()
                .insert(b.checksum.clone());
        }
    }
    sel
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bm(set: u64, md5: &str, sr: f64) -> Beatmap {
        Beatmap {
            id: 0,
            set_id: set,
            checksum: md5.into(),
            version: String::new(),
            sr,
        }
    }

    #[test]
    fn bucket_boundaries() {
        assert_eq!(bucket_of(0.0), 0);
        assert_eq!(bucket_of(0.99), 0);
        assert_eq!(bucket_of(6.999), 6);
        assert_eq!(bucket_of(7.0), 7);
        assert_eq!(bucket_of(8.99), 8);
        assert_eq!(bucket_of(9.99), 9);
        assert_eq!(bucket_of(10.0), 10);
        assert_eq!(bucket_of(14.3), 10);
        assert_eq!(bucket_of(-1.0), 0);
        assert_eq!(bucket_of(f64::NAN), 0);
    }

    #[test]
    fn histogram_counts_diffs_and_sets() {
        let maps = vec![
            bm(1, "a", 7.1),
            bm(1, "b", 7.9),
            bm(2, "c", 7.5),
            bm(2, "d", 8.2),
            bm(3, "e", 11.0),
        ];
        let h = histogram(&maps);
        assert_eq!(h.len(), 11);
        assert_eq!(
            h[7],
            Bucket {
                star: 7,
                diffs: 3,
                sets: 2
            }
        );
        assert_eq!(
            h[8],
            Bucket {
                star: 8,
                diffs: 1,
                sets: 1
            }
        );
        assert_eq!(
            h[10],
            Bucket {
                star: 10,
                diffs: 1,
                sets: 1
            }
        );
        assert_eq!(h[0].diffs, 0);
    }

    #[test]
    fn select_7_and_8_covers_7_00_to_8_99() {
        let maps = vec![
            bm(1, "a", 6.99),
            bm(1, "b", 7.0),
            bm(1, "c", 8.99),
            bm(2, "d", 9.0),
            bm(3, "e", 7.5),
        ];
        let sel = select(&maps, &[7, 8]);
        assert_eq!(sel.diff_count(), 3);
        assert_eq!(
            sel.by_set[&1],
            BTreeSet::from(["b".to_string(), "c".to_string()])
        );
        assert_eq!(sel.by_set[&3], BTreeSet::from(["e".to_string()]));
        assert!(!sel.by_set.contains_key(&2));
    }
}
