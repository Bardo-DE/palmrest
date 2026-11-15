//! The contact set a stroke follows and the one centroid-delta computation.

use crate::gestures::Sample;
use crate::touch::TouchId;

/// The ids of the running set, sorted, and where each contact was at the last report.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Follow {
    ids: Vec<TouchId>,
    last: Vec<Sample>,
}

impl Follow {
    /// Follows exactly these contacts from now on.
    pub fn land(contacts: &[Sample]) -> Self {
        Self {
            ids: sorted_ids(contacts),
            last: contacts.to_vec(),
        }
    }

    /// Whether `contacts` are the set being followed: the same ids, none twice.
    pub fn is(&self, contacts: &[Sample]) -> bool {
        let ids = sorted_ids(contacts);
        ids == self.ids && ids.windows(2).all(|w| w[0] != w[1])
    }

    /// How many contacts are followed.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// The mean displacement of the contacts since the last report, per axis in grid units,
    /// truncated toward zero. `contacts` must be the followed set.
    pub fn delta(self, contacts: &[Sample]) -> (Self, (i32, i32)) {
        let (sx, sy) = contacts
            .iter()
            .map(|c| c.at.delta_from(self.previous(c)))
            .fold((0, 0), |(x, y), d| (x + d.x, y + d.y));
        let n = contacts.len().max(1) as i32;
        let next = Self {
            ids: self.ids,
            last: contacts.to_vec(),
        };
        (next, (sx / n, sy / n))
    }

    fn previous(&self, c: &Sample) -> crate::touch::Point {
        self.last
            .iter()
            .find(|s| s.id == c.id)
            .map_or(c.at, |s| s.at)
    }
}

fn sorted_ids(contacts: &[Sample]) -> Vec<TouchId> {
    let mut ids: Vec<TouchId> = contacts.iter().map(|c| c.id).collect();
    ids.sort();
    ids
}
