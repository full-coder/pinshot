//! Pinned windows: the three ways a pin can disappear, and why they differ.
//!
//! `spec/06-pin-window.md` is the contract. The short version:
//!
//! * **close**   — gone, but the last `closed_limit` closed pins can be brought back;
//! * **hide all**— temporary, never counts against the restore queue;
//! * **destroy** — permanent.
//!
//! Getting this wrong is the single most common complaint about pin tools, so the
//! whole thing is a state machine with tests and no window code in sight.

use pinshot_core::Rect;
use std::collections::{BTreeMap, VecDeque};

pub type PinId = String;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinState {
    Visible,
    /// Hidden by "hide all", still restorable by "show all".
    Hidden,
    /// Closed; may be restored from the recoverable queue.
    Closed,
}

#[derive(Clone, Debug)]
pub struct Pin {
    pub id: PinId,
    pub capture_id: Option<String>,
    pub state: PinState,
    /// Position and size in points.
    pub bounds: Rect,
    pub scale: f32,
    pub opacity: f32,
    /// 0, 90, 180 or 270 degrees.
    pub rotation: u16,
    pub flip_h: bool,
    pub flip_v: bool,
    pub click_through: bool,
    pub thumbnail: bool,
    pub group: Option<String>,
    /// Monotonic counter, used to decide which closed pin to drop first.
    pub closed_at: Option<u64>,
}

impl Pin {
    pub fn size(&self) -> (u32, u32) {
        (self.bounds.w, self.bounds.h)
    }

    /// Resets scale and opacity — the middle-click action.
    pub fn reset(&mut self) {
        self.scale = 1.0;
        self.opacity = 1.0;
    }

    pub fn rotate(self, clockwise: bool) -> Self {
        let rotation = if clockwise {
            (self.rotation + 90) % 360
        } else {
            (self.rotation + 270) % 360
        };
        // Swap the footprint only when the orientation axis changes: 90 → 180 must
        // swap back, 90 → 270 must not swap a second time.
        let was_portrait = (self.rotation / 90) % 2 == 1;
        let is_portrait = (rotation / 90) % 2 == 1;
        let bounds = if was_portrait != is_portrait {
            Rect::new(self.bounds.x, self.bounds.y, self.bounds.h, self.bounds.w)
        } else {
            self.bounds
        };
        Self {
            rotation,
            bounds,
            ..self
        }
    }
}

pub struct PinManager {
    pins: BTreeMap<PinId, Pin>,
    /// Raise order, oldest first.
    order: Vec<PinId>,
    recoverable: VecDeque<PinId>,
    closed_limit: usize,
    next_id: u64,
    clock: u64,
}

impl Default for PinManager {
    fn default() -> Self {
        Self::new(1)
    }
}

impl PinManager {
    pub fn new(closed_limit: usize) -> Self {
        Self {
            pins: BTreeMap::new(),
            order: Vec::new(),
            recoverable: VecDeque::new(),
            closed_limit,
            next_id: 1,
            clock: 0,
        }
    }

    pub fn pins(&self) -> impl Iterator<Item = &Pin> {
        self.pins.values()
    }

    pub fn visible(&self) -> Vec<&Pin> {
        self.pins
            .values()
            .filter(|pin| pin.state == PinState::Visible)
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<&Pin> {
        self.pins.get(id)
    }

    pub fn len(&self) -> usize {
        self.pins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pins.is_empty()
    }

    /// How many closed pins can still be restored.
    pub fn recoverable(&self) -> usize {
        self.recoverable.len()
    }

    pub fn create(&mut self, bounds: Rect, capture_id: Option<String>) -> PinId {
        let id = format!("pin-{}", self.next_id);
        self.next_id += 1;
        let pin = Pin {
            id: id.clone(),
            capture_id,
            state: PinState::Visible,
            bounds,
            scale: 1.0,
            opacity: 1.0,
            rotation: 0,
            flip_h: false,
            flip_v: false,
            click_through: false,
            thumbnail: false,
            group: None,
            closed_at: None,
        };
        self.pins.insert(id.clone(), pin);
        self.order.push(id.clone());
        id
    }

    /// Closes a pin. Beyond `closed_limit` the oldest closed pin is destroyed.
    pub fn close(&mut self, id: &str) -> bool {
        self.clock += 1;
        let Some(pin) = self.pins.get_mut(id) else {
            return false;
        };
        if pin.state == PinState::Closed {
            return false;
        }
        pin.state = PinState::Closed;
        pin.closed_at = Some(self.clock);
        self.order.retain(|entry| entry != id);
        self.recoverable.push_back(id.to_owned());
        while self.recoverable.len() > self.closed_limit {
            if let Some(oldest) = self.recoverable.pop_front() {
                self.pins.remove(&oldest);
            }
        }
        true
    }

    /// Brings the most recently closed pin back. Returns its id.
    pub fn restore(&mut self) -> Option<PinId> {
        let id = self.recoverable.pop_back()?;
        let pin = self.pins.get_mut(&id)?;
        pin.state = PinState::Visible;
        pin.closed_at = None;
        self.order.push(id.clone());
        Some(id)
    }

    /// Permanent: removed from both the window list and the restore queue.
    pub fn destroy(&mut self, id: &str) -> bool {
        if self.pins.remove(id).is_none() {
            return false;
        }
        self.order.retain(|entry| entry != id);
        self.recoverable.retain(|entry| entry != id);
        true
    }

    /// Hides every visible pin. Does not touch the restore queue, by design.
    pub fn hide_all(&mut self) {
        for pin in self.pins.values_mut() {
            if pin.state == PinState::Visible {
                pin.state = PinState::Hidden;
            }
        }
        self.order.clear();
    }

    pub fn show_all(&mut self) {
        let mut shown = Vec::new();
        for pin in self.pins.values_mut() {
            if pin.state == PinState::Hidden {
                pin.state = PinState::Visible;
                shown.push(pin.id.clone());
            }
        }
        shown.sort();
        self.order.extend(shown);
    }

    /// Closes every pin in a group (closing a group is "close", not "destroy").
    pub fn close_group(&mut self, group: &str) -> usize {
        let ids: Vec<String> = self
            .pins
            .values()
            .filter(|pin| pin.group.as_deref() == Some(group) && pin.state != PinState::Closed)
            .map(|pin| pin.id.clone())
            .collect();
        let mut closed = 0;
        for id in ids {
            if self.close(&id) {
                closed += 1;
            }
        }
        closed
    }

    /// At exit: closed pins are gone for good, hidden ones survive.
    ///
    /// This is what stops a restart from resurrecting a pile of windows the user
    /// already dismissed, while still honouring "I only hid these for a moment".
    pub fn shutdown(&mut self) {
        let closed: Vec<String> = self
            .pins
            .values()
            .filter(|pin| pin.state == PinState::Closed)
            .map(|pin| pin.id.clone())
            .collect();
        for id in closed {
            self.pins.remove(&id);
        }
        self.recoverable.clear();
        self.order.clear();
    }

    pub fn set_opacity(&mut self, id: &str, opacity: f32) -> bool {
        match self.pins.get_mut(id) {
            Some(pin) => {
                pin.opacity = opacity.clamp(0.1, 1.0);
                true
            }
            None => false,
        }
    }

    /// Middle-click: back to 100% size and 100% opacity.
    pub fn reset(&mut self, id: &str) -> bool {
        match self.pins.get_mut(id) {
            Some(pin) => {
                pin.reset();
                true
            }
            None => false,
        }
    }

    pub fn set_thumbnail(&mut self, id: &str, thumbnail: bool) -> bool {
        match self.pins.get_mut(id) {
            Some(pin) => {
                pin.thumbnail = thumbnail;
                true
            }
            None => false,
        }
    }

    pub fn set_click_through(&mut self, id: &str, through: bool) -> bool {
        match self.pins.get_mut(id) {
            Some(pin) => {
                pin.click_through = through;
                true
            }
            None => false,
        }
    }

    pub fn rotate(&mut self, id: &str, clockwise: bool) -> bool {
        match self.pins.get_mut(id) {
            Some(pin) => {
                let rotated = pin.clone().rotate(clockwise);
                *pin = rotated;
                true
            }
            None => false,
        }
    }

    /// Topmost pin at a point, used by the click-through hotkey.
    pub fn topmost_at(&self, point: pinshot_core::Point) -> Option<&Pin> {
        self.order
            .iter()
            .rev()
            .filter_map(|id| self.pins.get(id))
            .find(|pin| pin.state == PinState::Visible && pin.bounds.contains(point))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::new(10, 10, 200, 100)
    }

    #[test]
    fn closing_then_restoring_brings_the_pin_back() {
        let mut manager = PinManager::default();
        let id = manager.create(rect(), None);
        assert_eq!(manager.visible().len(), 1);

        assert!(manager.close(&id));
        assert_eq!(manager.visible().len(), 0);
        assert_eq!(manager.recoverable(), 1);
        // Closing twice is a no-op, not a second queue entry.
        assert!(!manager.close(&id));
        assert_eq!(manager.recoverable(), 1);

        assert_eq!(manager.restore().as_deref(), Some(id.as_str()));
        assert_eq!(manager.visible().len(), 1);
        assert_eq!(manager.recoverable(), 0);
    }

    #[test]
    fn only_the_configured_number_of_closed_pins_survive() {
        let mut manager = PinManager::new(1);
        let first = manager.create(rect(), None);
        let second = manager.create(rect(), None);

        manager.close(&first);
        manager.close(&second);

        assert_eq!(manager.recoverable(), 1);
        assert!(
            manager.get(&first).is_none(),
            "oldest closed pin must be dropped"
        );
        assert!(manager.get(&second).is_some());
        assert_eq!(manager.restore().as_deref(), Some(second.as_str()));
    }

    #[test]
    fn destroy_is_permanent() {
        let mut manager = PinManager::default();
        let id = manager.create(rect(), None);
        manager.close(&id);
        assert!(manager.destroy(&id));
        assert!(manager.get(&id).is_none());
        assert_eq!(manager.restore(), None);
        assert!(!manager.destroy(&id));
    }

    #[test]
    fn hide_all_is_not_close() {
        let mut manager = PinManager::default();
        let id = manager.create(rect(), None);
        manager.close(&id);
        let other = manager.create(rect(), None);

        manager.hide_all();
        assert_eq!(manager.visible().len(), 0);
        // Hiding must not evict the closed pin from the restore queue.
        assert_eq!(manager.recoverable(), 1);
        assert_eq!(manager.get(&other).unwrap().state, PinState::Hidden);

        manager.show_all();
        assert_eq!(manager.visible().len(), 1);
        assert_eq!(manager.get(&other).unwrap().state, PinState::Visible);
    }

    #[test]
    fn shutdown_keeps_hidden_pins_and_drops_closed_ones() {
        let mut manager = PinManager::default();
        let closed = manager.create(rect(), None);
        let hidden = manager.create(rect(), None);
        let visible = manager.create(rect(), None);

        manager.close(&closed);
        manager.hide_all();
        manager.show_all(); // hidden comes back
        manager.hide_all();

        manager.shutdown();
        assert!(manager.get(&closed).is_none(), "closed pins die at exit");
        assert_eq!(manager.get(&hidden).unwrap().state, PinState::Hidden);
        assert_eq!(manager.get(&visible).unwrap().state, PinState::Hidden);
        // Nothing is restorable after a restart.
        assert_eq!(manager.restore(), None);
    }

    #[test]
    fn a_group_closes_together() {
        let mut manager = PinManager::new(10);
        let a = manager.create(rect(), None);
        let b = manager.create(rect(), None);
        let other = manager.create(rect(), None);
        for id in [&a, &b] {
            manager.pins.get_mut(id).unwrap().group = Some("对比".into());
        }

        assert_eq!(manager.close_group("对比"), 2);
        assert_eq!(manager.get(&a).unwrap().state, PinState::Closed);
        assert_eq!(manager.get(&b).unwrap().state, PinState::Closed);
        assert_eq!(manager.get(&other).unwrap().state, PinState::Visible);
    }

    #[test]
    fn rotation_swaps_the_footprint() {
        let mut manager = PinManager::default();
        let id = manager.create(rect(), None);
        assert!(manager.rotate(&id, true));
        let pin = manager.get(&id).unwrap();
        assert_eq!(pin.rotation, 90);
        assert_eq!((pin.bounds.w, pin.bounds.h), (100, 200));
        manager.rotate(&id, true);
        let pin = manager.get(&id).unwrap();
        assert_eq!(pin.rotation, 180);
        assert_eq!((pin.bounds.w, pin.bounds.h), (200, 100));
        manager.rotate(&id, false);
        assert_eq!(manager.get(&id).unwrap().rotation, 90);
    }

    #[test]
    fn opacity_is_clamped() {
        let mut manager = PinManager::default();
        let id = manager.create(rect(), None);
        manager.set_opacity(&id, 0.0);
        assert!((manager.get(&id).unwrap().opacity - 0.1).abs() < 1e-6);
        manager.set_opacity(&id, 5.0);
        assert!((manager.get(&id).unwrap().opacity - 1.0).abs() < 1e-6);
        manager.set_opacity(&id, 0.3);
        manager.pins.get_mut(&id).unwrap().scale = 2.0;
        assert!(manager.reset(&id));
        assert!((manager.get(&id).unwrap().opacity - 1.0).abs() < 1e-6);
        assert!((manager.get(&id).unwrap().scale - 1.0).abs() < 1e-6);
    }

    #[test]
    fn topmost_pin_wins() {
        let mut manager = PinManager::default();
        let lower = manager.create(Rect::new(0, 0, 100, 100), None);
        let upper = manager.create(Rect::new(50, 50, 100, 100), None);
        let hit = manager
            .topmost_at(pinshot_core::Point::new(60, 60))
            .unwrap();
        assert_eq!(hit.id, upper);
        let hit = manager
            .topmost_at(pinshot_core::Point::new(10, 10))
            .unwrap();
        assert_eq!(hit.id, lower);
        assert!(manager
            .topmost_at(pinshot_core::Point::new(500, 500))
            .is_none());
    }
}
