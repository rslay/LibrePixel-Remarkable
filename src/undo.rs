//! Snapshot undo: every edit pushes the sprite as it was before. Sprites here are small
//! (a few hundred pixels square, a handful of layers), so whole copies are cheap and
//! trivially correct. A byte budget drops the oldest entries.

use crate::doc::Sprite;

pub struct Undo {
    past: Vec<Sprite>,
    future: Vec<Sprite>,
    budget: usize,
}

fn cost(s: &Sprite) -> usize {
    s.layers.iter().map(|l| l.image.px.len() + 64).sum::<usize>() + 128
}

impl Undo {
    pub fn new(budget_bytes: usize) -> Undo {
        Undo { past: Vec::new(), future: Vec::new(), budget: budget_bytes }
    }

    /// Call before changing the sprite, with its current state.
    pub fn record(&mut self, before: &Sprite) {
        self.future.clear();
        self.past.push(before.clone());
        let mut total: usize = self.past.iter().map(cost).sum();
        while self.past.len() > 1 && total > self.budget {
            total -= cost(&self.past.remove(0));
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// Swap the current sprite for the previous one.
    pub fn undo(&mut self, current: &Sprite) -> Option<Sprite> {
        let prev = self.past.pop()?;
        self.future.push(current.clone());
        Some(prev)
    }

    pub fn redo(&mut self, current: &Sprite) -> Option<Sprite> {
        let next = self.future.pop()?;
        self.past.push(current.clone());
        Some(next)
    }

    pub fn clear(&mut self) {
        self.past.clear();
        self.future.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo() {
        let mut u = Undo::new(1 << 20);
        let a = Sprite::new(2, 2);
        let mut b = a.clone();
        u.record(&a);
        b.layers[0].image.set(0, 0, 5);
        assert!(u.can_undo() && !u.can_redo());
        let back = u.undo(&b).unwrap();
        assert_eq!(back, a);
        assert!(u.can_redo());
        assert_eq!(u.redo(&back).unwrap(), b);
        assert!(u.undo(&b).is_some());
        assert!(u.undo(&a).is_none());
    }

    #[test]
    fn budget_drops_oldest() {
        let mut u = Undo::new(1); // tiny: keeps exactly one entry
        let a = Sprite::new(4, 4);
        u.record(&a);
        u.record(&a);
        assert_eq!(u.past.len(), 1);
    }
}
