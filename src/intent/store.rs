use super::{MoveIntent, MoveIntentDecodeError};
use crate::key::{Bucket, KeyWriter, Ty};
use crate::model::inode::Ino;
use antidotec::{rwset, RawIdent, Transaction};
use std::mem;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum IntentStoreError {
    #[error("failed to access the intent store: {0}")]
    Antidote(#[from] antidotec::Error),
    #[error("failed to decode a stored move intent: {0}")]
    Decode(#[from] MoveIntentDecodeError),
}

#[derive(Debug, Copy, Clone)]
pub(crate) struct IntentStore {
    bucket: Bucket,
}

impl IntentStore {
    pub(crate) fn new(bucket: Bucket) -> Self {
        Self { bucket }
    }

    pub(crate) async fn put(
        &self,
        tx: &mut Transaction<'_>,
        intent: &MoveIntent,
    ) -> Result<(), antidotec::Error> {
        let update = rwset::insert(Key::new(intent.ino))
            .add(intent.to_bytes())
            .build();
        tx.update(self.bucket, std::iter::once(update)).await?;
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) async fn load(
        &self,
        tx: &mut Transaction<'_>,
        ino: Ino,
    ) -> Result<Vec<MoveIntent>, IntentStoreError> {
        let mut reply = tx
            .read(self.bucket, std::iter::once(rwset::get(Key::new(ino))))
            .await?;
        let encoded = match reply.rwset(0) {
            Some(encoded) => encoded,
            None => return Ok(Vec::new()),
        };

        let mut intents = encoded
            .into_iter()
            .map(|bytes| MoveIntent::from_bytes(&bytes))
            .collect::<Result<Vec<_>, _>>()
            .map_err(IntentStoreError::from)?;
        intents.sort_by_key(|intent| {
            (
                intent.op_id.timestamp,
                intent.op_id.actor,
                intent.op_id.sequence,
            )
        });
        Ok(intents)
    }

    #[allow(dead_code)]
    pub(crate) async fn remove(
        &self,
        tx: &mut Transaction<'_>,
        intent: &MoveIntent,
    ) -> Result<(), antidotec::Error> {
        let update = rwset::remove(Key::new(intent.ino))
            .remove(intent.to_bytes())
            .build();
        tx.update(self.bucket, std::iter::once(update)).await?;
        Ok(())
    }
}

#[derive(Debug, Copy, Clone)]
struct Key {
    ino: Ino,
}

impl Key {
    fn new(ino: Ino) -> Self {
        Self { ino }
    }
}

impl Into<RawIdent> for Key {
    fn into(self) -> RawIdent {
        KeyWriter::with_capacity(Ty::MoveIntentSet, mem::size_of::<u64>())
            .write_u64(self.ino.into())
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::IntentStore;
    use crate::key::Bucket;
    use crate::model::inode::Ino;
    use crate::time;
    use crate::view::{Name, NameRef, View};
    use crate::intent::MoveIntent;
    use antidotec::Connection;

    #[tokio::test]
    #[ignore]
    async fn persists_loads_and_removes_move_intent() -> Result<(), Box<dyn std::error::Error>> {
        let address = std::env::var("ELMERFS_TEST_ANTIDOTE")
            .unwrap_or_else(|_| "127.0.0.1:8101".into());
        let mut connection = Connection::new(&address).await?;
        let store = IntentStore::new(Bucket::new(u32::MAX));

        let timestamp = time::ts(time::now());
        let ino = Ino(timestamp.as_secs() ^ ((timestamp.subsec_nanos() as u64) << 32));
        let intent = MoveIntent::new(
            u8::MAX,
            ino,
            Ino(1),
            &NameRef::Exact(Name::new("before", View { uid: 1000 })),
            Ino(2),
            &NameRef::Exact(Name::new("after", View { uid: 1000 })),
        );

        let mut tx = connection.transaction().await?;
        store.put(&mut tx, &intent).await?;
        tx.commit().await?;

        let mut tx = connection.transaction().await?;
        let loaded = store.load(&mut tx, ino).await?;
        tx.commit().await?;
        assert_eq!(loaded, vec![intent.clone()]);

        let mut tx = connection.transaction().await?;
        store.remove(&mut tx, &intent).await?;
        tx.commit().await?;

        let mut tx = connection.transaction().await?;
        let loaded = store.load(&mut tx, ino).await?;
        tx.commit().await?;
        assert!(loaded.is_empty());

        Ok(())
    }
}
