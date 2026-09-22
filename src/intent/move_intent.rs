use crate::model::inode::Ino;
use crate::view::{Name, NameRef, View};
use antidotec::Bytes;
use std::convert::TryInto;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use thiserror::Error;

static NEXT_OP_SEQUENCE: AtomicU64 = AtomicU64::new(1);
const ENCODING_VERSION: u8 = 1;

pub(crate) type ActorId = u8;
pub(crate) type Timestamp = Duration;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(crate) struct OpId {
    pub(crate) actor: ActorId,
    pub(crate) timestamp: Timestamp,
    pub(crate) sequence: u64,
}

impl OpId {
    fn next(actor: ActorId, timestamp: Timestamp) -> Self {
        Self {
            actor,
            timestamp,
            sequence: NEXT_OP_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
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
        let timestamp = crate::time::ts(crate::time::now());
        Self {
            op_id: OpId::next(actor, timestamp),
            actor,
            timestamp,
            ino,
            old_parent,
            old_name: old_name.clone(),
            new_parent,
            new_name: new_name.clone(),
        }
    }

    pub(super) fn to_bytes(&self) -> Bytes {
        debug_assert_eq!(self.op_id.actor, self.actor);
        debug_assert_eq!(self.op_id.timestamp, self.timestamp);

        let mut buffer = Vec::new();
        buffer.push(ENCODING_VERSION);
        buffer.push(self.actor);
        write_duration(&mut buffer, self.timestamp);
        buffer.extend_from_slice(&self.op_id.sequence.to_le_bytes());
        buffer.extend_from_slice(&self.ino.to_le_bytes());
        buffer.extend_from_slice(&self.old_parent.to_le_bytes());
        write_name_ref(&mut buffer, &self.old_name);
        buffer.extend_from_slice(&self.new_parent.to_le_bytes());
        write_name_ref(&mut buffer, &self.new_name);
        Bytes::from(buffer)
    }

    pub(super) fn from_bytes(bytes: &[u8]) -> Result<Self, MoveIntentDecodeError> {
        let mut decoder = Decoder::new(bytes);
        let version = decoder.read_u8()?;
        if version != ENCODING_VERSION {
            return Err(MoveIntentDecodeError::UnsupportedVersion(version));
        }

        let actor = decoder.read_u8()?;
        let timestamp = decoder.read_duration()?;
        let sequence = decoder.read_u64()?;
        let ino = Ino(decoder.read_u64()?);
        let old_parent = Ino(decoder.read_u64()?);
        let old_name = decoder.read_name_ref()?;
        let new_parent = Ino(decoder.read_u64()?);
        let new_name = decoder.read_name_ref()?;
        decoder.finish()?;

        Ok(Self {
            op_id: OpId {
                actor,
                timestamp,
                sequence,
            },
            actor,
            timestamp,
            ino,
            old_parent,
            old_name,
            new_parent,
            new_name,
        })
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub(crate) enum MoveIntentDecodeError {
    #[error("move intent payload ended unexpectedly")]
    UnexpectedEof,
    #[error("unsupported move intent encoding version {0}")]
    UnsupportedVersion(u8),
    #[error("invalid move intent name kind {0}")]
    InvalidNameKind(u8),
    #[error("move intent name is not valid UTF-8")]
    InvalidUtf8,
    #[error("move intent name length does not fit this platform")]
    NameTooLong,
    #[error("move intent payload has trailing bytes")]
    TrailingBytes,
}

fn write_duration(buffer: &mut Vec<u8>, duration: Duration) {
    buffer.extend_from_slice(&duration.as_secs().to_le_bytes());
    buffer.extend_from_slice(&duration.subsec_nanos().to_le_bytes());
}

fn write_name_ref(buffer: &mut Vec<u8>, name: &NameRef) {
    let (kind, view, prefix) = match name {
        NameRef::Partial(prefix) => (0, 0, prefix),
        NameRef::Exact(name) => (1, name.view.uid, &name.prefix),
    };

    buffer.push(kind);
    buffer.extend_from_slice(&view.to_le_bytes());
    buffer.extend_from_slice(&(prefix.len() as u64).to_le_bytes());
    buffer.extend_from_slice(prefix.as_bytes());
}

struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Decoder<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn read_u8(&mut self) -> Result<u8, MoveIntentDecodeError> {
        Ok(self.take(1)?[0])
    }

    fn read_u32(&mut self) -> Result<u32, MoveIntentDecodeError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn read_u64(&mut self) -> Result<u64, MoveIntentDecodeError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn read_duration(&mut self) -> Result<Duration, MoveIntentDecodeError> {
        Ok(Duration::new(self.read_u64()?, self.read_u32()?))
    }

    fn read_name_ref(&mut self) -> Result<NameRef, MoveIntentDecodeError> {
        let kind = self.read_u8()?;
        let view = self.read_u32()?;
        let len = self
            .read_u64()?
            .try_into()
            .map_err(|_| MoveIntentDecodeError::NameTooLong)?;
        let prefix = String::from_utf8(self.take(len)?.to_vec())
            .map_err(|_| MoveIntentDecodeError::InvalidUtf8)?;

        match kind {
            0 => Ok(NameRef::Partial(prefix)),
            1 => Ok(NameRef::Exact(Name::new(prefix, View { uid: view }))),
            _ => Err(MoveIntentDecodeError::InvalidNameKind(kind)),
        }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], MoveIntentDecodeError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(MoveIntentDecodeError::UnexpectedEof)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(MoveIntentDecodeError::UnexpectedEof)?;
        self.offset = end;
        Ok(value)
    }

    fn finish(self) -> Result<(), MoveIntentDecodeError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(MoveIntentDecodeError::TrailingBytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MoveIntent, MoveIntentDecodeError};
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
        assert_eq!(intent.op_id.timestamp, intent.timestamp);
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

    #[test]
    fn encoding_round_trips_partial_and_exact_names() {
        let intent = MoveIntent::new(
            7,
            Ino(42),
            Ino(1),
            &NameRef::Partial("before".into()),
            Ino(2),
            &NameRef::Exact(crate::view::Name::new(
                "after",
                crate::view::View { uid: 1000 },
            )),
        );

        assert_eq!(MoveIntent::from_bytes(&intent.to_bytes()), Ok(intent));
    }

    #[test]
    fn decoding_rejects_unknown_versions() {
        assert_eq!(
            MoveIntent::from_bytes(&[2]),
            Err(MoveIntentDecodeError::UnsupportedVersion(2))
        );
    }
}
