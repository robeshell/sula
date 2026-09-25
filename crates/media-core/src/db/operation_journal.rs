use super::{AppDatabase, DatabaseError};

impl AppDatabase {
    pub fn create_media_operation(&self, id: &str, payload: &str) -> Result<(), DatabaseError> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO media_operation_journal(id,payload) VALUES (?1,?2)",
                [id, payload],
            )?;
            Ok(())
        })
    }
    pub fn update_media_operation(&self, id: &str, payload: &str) -> Result<(), DatabaseError> {
        self.with_conn(|conn| {
            if conn.execute(
                "UPDATE media_operation_journal SET payload=?2 WHERE id=?1 AND committed=0",
                [id, payload],
            )? != 1
            {
                return Err(DatabaseError::Io(std::io::Error::other(
                    "pending media operation missing",
                )));
            }
            Ok(())
        })
    }
    pub fn media_operations(&self) -> Result<Vec<(String, String, bool)>, DatabaseError> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id,payload,committed FROM media_operation_journal ORDER BY rowid",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
    }
    pub fn remove_media_operation(&self, id: &str) -> Result<(), DatabaseError> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM media_operation_journal WHERE id=?1", [id])?;
            Ok(())
        })
    }
}
