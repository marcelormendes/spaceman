//! Pure input state: previews may be skipped, but completed gestures and key
//! commands retain their arrival order until dispatched or explicitly cleared.
use std::collections::VecDeque;

use super::{FinalGesture, WorkerAction};
use crate::{PhysicalWindowId, Rect};

const MAX_PENDING_INPUTS: usize = 64;

pub(super) fn is_window_drag(start: Rect, current: Rect) -> bool {
    let moved = (start.x - current.x).abs().max((start.y - current.y).abs()) >= 6;
    let resized = (start.width - current.width)
        .abs()
        .max((start.height - current.height).abs())
        > 2;
    moved && !resized
}

pub(super) fn is_window_resize(start: Rect, current: Rect) -> bool {
    (start.width - current.width)
        .abs()
        .max((start.height - current.height).abs())
        >= 6
}

/// Whether a three-finger workspace swipe must be ignored because a pointer
/// button is down or a window drag/resize gesture is in progress (such as
/// macOS Accessibility "Three Finger Drag").
pub(super) fn should_ignore_swipe(
    mouse_down: bool,
    mouse_was_down: bool,
    gesture_active: bool,
) -> bool {
    mouse_down || mouse_was_down || gesture_active
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum QueuedInput {
    Action(WorkerAction),
    Gesture(FinalGesture),
}

#[derive(Default)]
pub(super) struct InputQueue(VecDeque<QueuedInput>);

impl InputQueue {
    pub(super) fn push(&mut self, input: QueuedInput) -> Result<(), &'static str> {
        if self.0.len() >= MAX_PENDING_INPUTS {
            return Err("Input queue full; command was not queued");
        }
        self.0.push_back(input);
        Ok(())
    }

    pub(super) fn pop(&mut self) -> Option<QueuedInput> {
        self.0.pop_front()
    }

    pub(super) fn front(&self) -> Option<&QueuedInput> {
        self.0.front()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(super) fn clear(&mut self) {
        self.0.clear();
    }
}

#[derive(Default)]
pub(super) struct GestureTracker {
    active: Option<Gesture>,
}

struct Gesture {
    window: PhysicalWindowId,
    start: Rect,
    dragged: bool,
    resized: bool,
    last_preview: Option<FinalGesture>,
}

impl GestureTracker {
    pub(super) fn begin(&mut self, window: Option<(PhysicalWindowId, Rect)>) {
        self.active = window.map(|(window, start)| Gesture {
            window,
            start,
            dragged: false,
            resized: false,
            last_preview: None,
        });
    }

    pub(super) fn window(&self) -> Option<&PhysicalWindowId> {
        self.active.as_ref().map(|gesture| &gesture.window)
    }

    pub(super) fn clear(&mut self) {
        self.active = None;
    }

    pub(super) fn preview(
        &mut self,
        frame: Rect,
        point: Option<(i32, i32)>,
    ) -> Option<FinalGesture> {
        let gesture = self.active.as_mut()?;
        let next = gesture.classify(frame, point)?;
        if gesture.last_preview.as_ref() == Some(&next) {
            return None;
        }
        gesture.last_preview = Some(next.clone());
        Some(next)
    }

    pub(super) fn finish(
        &mut self,
        frame: Option<Rect>,
        point: Option<(i32, i32)>,
    ) -> Option<FinalGesture> {
        let mut gesture = self.active.take()?;
        // The final sample, not the existence of a preview tick, establishes
        // intent. Missing final evidence never replays an old preview frame.
        gesture.classify(frame?, point)
    }
}

impl Gesture {
    fn classify(&mut self, frame: Rect, point: Option<(i32, i32)>) -> Option<FinalGesture> {
        self.resized |= is_window_resize(self.start, frame);
        self.dragged |= is_window_drag(self.start, frame);
        if self.resized {
            Some(FinalGesture::Resize(self.window.clone(), frame))
        } else if self.dragged {
            point.map(|point| FinalGesture::Drop(self.window.clone(), point))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppInstanceId, Direction, ResizeDirection, WorkspaceId};

    fn window() -> PhysicalWindowId {
        PhysicalWindowId {
            app: AppInstanceId {
                pid: 1,
                launched_at: 1,
            },
            window_id: 10,
        }
    }

    fn start() -> Rect {
        Rect {
            x: 50,
            y: 20,
            width: 400,
            height: 300,
        }
    }

    #[test]
    fn busy_input_preserves_relative_commands_and_mixed_order() {
        let inputs = [
            WorkerAction::Resize(ResizeDirection::Grow),
            WorkerAction::Resize(ResizeDirection::Grow),
            WorkerAction::Resize(ResizeDirection::Grow),
            WorkerAction::SwitchWorkspace(WorkspaceId::new(2).unwrap()),
            WorkerAction::SwitchWorkspace(WorkspaceId::new(3).unwrap()),
            WorkerAction::ToggleLayout,
            WorkerAction::ToggleLayout,
            WorkerAction::Focus(Direction::Right),
            WorkerAction::CycleFocus,
        ]
        .map(QueuedInput::Action);
        let mut queue = InputQueue::default();
        for input in &inputs {
            queue.push(input.clone()).unwrap();
        }
        for input in inputs {
            assert_eq!(queue.pop(), Some(input));
        }
        assert!(queue.is_empty());
    }

    #[test]
    fn queue_overflow_is_explicit_and_never_overwrites_accepted_input() {
        let mut queue = InputQueue::default();
        for _ in 0..MAX_PENDING_INPUTS {
            queue
                .push(QueuedInput::Action(WorkerAction::CycleFocus))
                .unwrap();
        }
        assert!(
            queue
                .push(QueuedInput::Action(WorkerAction::ToggleLayout))
                .is_err()
        );
        for _ in 0..MAX_PENDING_INPUTS {
            assert_eq!(
                queue.pop(),
                Some(QueuedInput::Action(WorkerAction::CycleFocus))
            );
        }
        assert!(queue.is_empty());
    }

    #[test]
    fn failed_dispatch_retains_the_head_until_it_can_be_sent() {
        let mut queue = InputQueue::default();
        let first = QueuedInput::Action(WorkerAction::CycleFocus);
        queue.push(first.clone()).unwrap();
        // No displays or an unavailable worker must not consume the intent.
        assert_eq!(queue.front(), Some(&first));
        queue
            .push(QueuedInput::Action(WorkerAction::Balance))
            .unwrap();
        assert_eq!(queue.front(), Some(&first));
        assert_eq!(queue.pop(), Some(first));
        assert_eq!(
            queue.pop(),
            Some(QueuedInput::Action(WorkerAction::Balance))
        );
    }

    #[test]
    fn entire_gesture_during_busy_work_survives_without_a_preview_tick() {
        for end in [
            Rect { x: 1500, ..start() },
            Rect {
                width: 550,
                ..start()
            },
        ] {
            let mut tracker = GestureTracker::default();
            let mut queue = InputQueue::default();
            queue
                .push(QueuedInput::Action(WorkerAction::CycleFocus))
                .unwrap();
            tracker.begin(Some((window(), start())));
            // Worker stays busy from mouse-down through mouse-up: no preview.
            let completed = tracker.finish(Some(end), Some((1700, 100))).unwrap();
            let expected = if end.width != start().width {
                FinalGesture::Resize(window(), end)
            } else {
                FinalGesture::Drop(window(), (1700, 100))
            };
            assert_eq!(completed, expected);
            queue.push(QueuedInput::Gesture(completed)).unwrap();
            queue
                .push(QueuedInput::Action(WorkerAction::Balance))
                .unwrap();
            assert_eq!(
                queue.pop(),
                Some(QueuedInput::Action(WorkerAction::CycleFocus))
            );
            assert_eq!(queue.pop(), Some(QueuedInput::Gesture(expected)));
            assert_eq!(
                queue.pop(),
                Some(QueuedInput::Action(WorkerAction::Balance))
            );
            assert!(tracker.window().is_none());
        }
    }

    #[test]
    fn busy_preview_worker_queues_final_cross_display_drop() {
        let mut tracker = GestureTracker::default();
        tracker.begin(Some((window(), start())));
        assert!(
            tracker
                .preview(Rect { x: 500, ..start() }, Some((700, 400)))
                .is_some()
        );
        let drop = tracker
            .finish(Some(Rect { x: 1500, ..start() }), Some((1700, 400)))
            .unwrap();
        let mut queue = InputQueue::default();
        queue.push(QueuedInput::Gesture(drop)).unwrap();
        assert_eq!(
            queue.pop(),
            Some(QueuedInput::Gesture(FinalGesture::Drop(
                window(),
                (1700, 400)
            )))
        );
    }

    #[test]
    fn mouse_up_uses_final_resize_even_when_preview_was_already_sent() {
        let mut tracker = GestureTracker::default();
        tracker.begin(Some((window(), start())));
        let preview = Rect {
            width: 450,
            ..start()
        };
        assert!(tracker.preview(preview, None).is_some());
        assert!(tracker.preview(preview, None).is_none());
        let end = Rect {
            width: 600,
            ..start()
        };
        assert_eq!(
            tracker.finish(Some(end), None),
            Some(FinalGesture::Resize(window(), end))
        );
    }

    #[test]
    fn clicks_missing_readback_and_pause_do_not_replay_gestures() {
        let mut tracker = GestureTracker::default();
        tracker.begin(Some((window(), start())));
        assert_eq!(tracker.finish(Some(start()), Some((60, 40))), None);
        tracker.begin(Some((window(), start())));
        assert!(
            tracker
                .preview(
                    Rect {
                        width: 600,
                        ..start()
                    },
                    None
                )
                .is_some()
        );
        assert_eq!(tracker.finish(None, None), None);
        tracker.begin(Some((window(), start())));
        tracker.clear();
        assert_eq!(
            tracker.finish(Some(Rect { x: 1500, ..start() }), Some((1700, 100))),
            None
        );
        let mut queue = InputQueue::default();
        queue
            .push(QueuedInput::Action(WorkerAction::CycleFocus))
            .unwrap();
        queue.clear();
        assert_eq!(queue.pop(), None);
    }

    #[test]
    fn three_finger_swipes_are_ignored_while_a_pointer_button_or_drag_is_active() {
        let mut tracker = GestureTracker::default();
        // Normal three-finger swipe: no mouse button down, no active gesture.
        assert!(!should_ignore_swipe(
            false,
            false,
            tracker.window().is_some()
        ));

        // Left button pressed (e.g. macOS Three Finger Drag starts dragging a window).
        assert!(should_ignore_swipe(true, false, tracker.window().is_some()));

        // Active window drag tracked in GestureTracker suppresses swipes even
        // on the boundary tick where mouse_down just transitioned.
        tracker.begin(Some((window(), start())));
        assert!(should_ignore_swipe(true, true, tracker.window().is_some()));
        assert!(should_ignore_swipe(false, true, tracker.window().is_some()));
        assert!(should_ignore_swipe(
            false,
            false,
            tracker.window().is_some()
        ));

        // Finishing the drag clears GestureTracker so subsequent normal
        // three-finger swipes are accepted again.
        let moved = Rect { x: 200, ..start() };
        assert_eq!(
            tracker.finish(Some(moved), Some((260, 40))),
            Some(FinalGesture::Drop(window(), (260, 40)))
        );
        assert!(!should_ignore_swipe(
            false,
            false,
            tracker.window().is_some()
        ));
    }

    #[test]
    fn suppressed_swipes_never_enter_the_input_queue_while_normal_swipes_do() {
        let mut queue = InputQueue::default();
        let swipe = WorkerAction::Swipe(crate::swipe::Swipe::Right, (200, 100));

        // While dragging with three fingers (mouse_down = true), a swipe is dropped.
        if !should_ignore_swipe(true, true, true) {
            queue.push(QueuedInput::Action(swipe.clone())).unwrap();
        }
        assert!(queue.is_empty());

        // Ordinary three-finger swipe with no drag in progress is queued normally.
        if !should_ignore_swipe(false, false, false) {
            queue.push(QueuedInput::Action(swipe.clone())).unwrap();
        }
        assert_eq!(queue.pop(), Some(QueuedInput::Action(swipe)));
    }
}
