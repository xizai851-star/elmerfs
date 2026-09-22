use crate::model::inode::Ino;
use crate::view::NameRef;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static NEXT_OP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub(crate) type ActorId = u8;
pub(crate) type Timestamp = Duration;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(crate) struct OpId {
    pub(crate) actor: ActorId,
    pub(crate) sequence: u64,
}

impl OpId {
    fn next(actor: ActorId) -> Self {
        Self {
            actor,
            sequence: NEXT_OP_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MoveIntent {
    pub(crate) op_id: OpId,
    pub(crate) actor: ActorId,
    pub(crate) timestamp: Timestamp,
    pub(crate) ino: Ino,
    pub(crate) old_parent: Ino,
    pub(crate) old_name: NameRef,
    pub(crate) new_parent: Ino,
    pub(crate) new_name: NameRef,
}

impl MoveIntent {
    pub(crate) fn new(
        actor: ActorId,
        ino: Ino,
        old_parent: Ino,
        old_name: &NameRef,
        new_parent: Ino,
        new_name: &NameRef,
    ) -> Self {
        Self {
            op_id: OpId::next(actor),
            actor,
            timestamp: crate::time::ts(crate::time::now()),
            ino,
            old_parent,
            old_name: old_name.clone(),
            new_parent,
            new_name: new_name.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MoveIntent;
    use crate::model::inode::Ino;
    use crate::view::NameRef;

    #[test]
    fn captures_move_metadata() {
        let intent = MoveIntent::new(
            7,
            Ino(42),
            Ino(1),
            &NameRef::Partial("before".into()),
            Ino(2),
            &NameRef::Partial("after".into()),
        );

        assert_eq!(intent.op_id.actor, 7);
        assert_eq!(intent.actor, 7);
        assert_eq!(intent.ino, Ino(42));
        assert_eq!(intent.old_parent, Ino(1));
        assert_eq!(intent.new_parent, Ino(2));
        assert!(intent.timestamp <= crate::time::ts(crate::time::now()));

        match intent.old_name {
            NameRef::Partial(name) => assert_eq!(name, "before"),
            NameRef::Exact(_) => panic!("expected a partial old name"),
        }
        match intent.new_name {
            NameRef::Partial(name) => assert_eq!(name, "after"),
            NameRef::Exact(_) => panic!("expected a partial new name"),
        }
    }

    #[test]
    fn allocates_unique_operation_ids() {
        let old_name = NameRef::Partial("before".into());
        let new_name = NameRef::Partial("after".into());
        let first = MoveIntent::new(7, Ino(42), Ino(1), &old_name, Ino(2), &new_name);
        let second = MoveIntent::new(7, Ino(42), Ino(1), &old_name, Ino(2), &new_name);

        assert_ne!(first.op_id, second.op_id);
    }
}
