use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Unsupported backup length {0}; expected 540 bytes or 572 with signature")]
    Unsupported(usize),
    #[error("Invalid NTAG215 structure: {0}")]
    Invalid(&'static str),
    #[error("No such tag")]
    Missing,
    #[error("This backup is already in the library")]
    Duplicate,
    #[error("Database: {0}")]
    Sql(#[from] rusqlite::Error),
}

#[derive(Clone)]
pub struct Tag {
    pub pages: [u8; 540],
    pub uid: [u8; 7],
}

impl Tag {
    pub fn parse(input: &[u8]) -> Result<Self, Error> {
        if !matches!(input.len(), 540 | 572) {
            return Err(Error::Unsupported(input.len()));
        }
        let pages: [u8; 540] = input[..540].try_into().unwrap();
        if pages[0] != 4
            || pages[3] != (0x88 ^ pages[0] ^ pages[1] ^ pages[2])
            || pages[8] != pages[4..8].iter().fold(0, |a, b| a ^ b)
        {
            return Err(Error::Invalid("manufacturer UID or BCC"));
        }
        if pages[10..16] != [0x0f, 0xe0, 0xf1, 0x10, 0xff, 0xee]
            || pages[520..523] != [1, 0, 15]
            || pages[524..528] != [0, 0, 0, 4]
            || pages[528..532] != [0x5f, 0, 0, 0]
        {
            return Err(Error::Invalid(
                "lock bits, capability container or configuration",
            ));
        }
        Ok(Self {
            uid: [
                pages[0], pages[1], pages[2], pages[4], pages[5], pages[6], pages[7],
            ],
            pages,
        })
    }
}

#[derive(Serialize)]
pub struct Entry {
    pub id: i64,
    pub name: String,
    pub uid: String,
    pub figure: String,
    pub category: String,
    pub favorite: bool,
    pub selected: bool,
}
#[derive(Serialize)]
pub struct Scan {
    pub time: i64,
    pub name: String,
    pub outcome: String,
    pub simulation: bool,
}

pub struct Library(Connection);
impl Library {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let db = Connection::open(path)?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
          CREATE TABLE IF NOT EXISTS tags(id INTEGER PRIMARY KEY AUTOINCREMENT,name TEXT NOT NULL,
            uid TEXT NOT NULL, figure TEXT NOT NULL, data BLOB NOT NULL UNIQUE,
            category TEXT NOT NULL DEFAULT '',favorite INTEGER NOT NULL DEFAULT 0);
          CREATE TABLE IF NOT EXISTS selection(slot INTEGER PRIMARY KEY CHECK(slot=1),
            tag INTEGER REFERENCES tags(id) ON DELETE SET NULL);
          CREATE TABLE IF NOT EXISTS scans(id INTEGER PRIMARY KEY,time INTEGER NOT NULL,
            name TEXT NOT NULL,outcome TEXT NOT NULL,simulation INTEGER NOT NULL);",
        )?;
        Ok(Self(db))
    }

    pub fn add(&self, name: &str, data: &[u8]) -> Result<i64, Error> {
        let tag = Tag::parse(data)?;
        check_text(name, true)?;
        let count = self.0.execute(
            "INSERT INTO tags(name,uid,figure,data) VALUES(?,?,?,?) ON CONFLICT(data) DO NOTHING",
            params![
                name.trim(),
                hex::encode_upper(tag.uid),
                hex::encode_upper(&tag.pages[84..92]),
                data
            ],
        )?;
        if count == 0 {
            return Err(Error::Duplicate);
        }
        Ok(self.0.last_insert_rowid())
    }
    pub fn list(&self) -> Result<Vec<Entry>, Error> {
        let mut stmt=self.0.prepare("SELECT id,name,uid,figure,category,favorite,id IS (SELECT tag FROM selection WHERE slot=1) FROM tags ORDER BY name COLLATE NOCASE,id")?;
        let rows = stmt.query_map([], |r| {
            Ok(Entry {
                id: r.get(0)?,
                name: r.get(1)?,
                uid: r.get(2)?,
                figure: r.get(3)?,
                category: r.get(4)?,
                favorite: r.get(5)?,
                selected: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
    pub fn edit(&self, id: i64, category: &str, favorite: bool) -> Result<(), Error> {
        check_text(category, false)?;
        changed(self.0.execute(
            "UPDATE tags SET category=?,favorite=? WHERE id=?",
            params![category.trim(), favorite, id],
        )?)
    }
    pub fn delete(&self, id: i64) -> Result<(), Error> {
        changed(self.0.execute("DELETE FROM tags WHERE id=?", [id])?)
    }
    pub fn select(&self, id: i64) -> Result<(), Error> {
        changed(self.0.execute(
            "INSERT OR REPLACE INTO selection(slot,tag) SELECT 1,id FROM tags WHERE id=?",
            [id],
        )?)
    }
    pub fn selected(&self) -> Result<(String, Tag), Error> {
        let (name, data): (String, Vec<u8>) = self
            .0
            .query_row(
                "SELECT name,data FROM tags JOIN selection ON tags.id=selection.tag",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or(Error::Missing)?;
        Ok((name, Tag::parse(&data)?))
    }
    pub fn record(&mut self, name: &str, outcome: &str, simulation: bool) -> Result<(), Error> {
        let tx = self.0.transaction()?;
        tx.execute(
            "INSERT INTO scans(time,name,outcome,simulation) VALUES(unixepoch(),?,?,?)",
            params![name, outcome, simulation],
        )?;
        tx.execute(
            "DELETE FROM scans WHERE id <= (SELECT max(id)-1000 FROM scans)",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn history(&self) -> Result<Vec<Scan>, Error> {
        let mut stmt = self
            .0
            .prepare("SELECT time,name,outcome,simulation FROM scans ORDER BY id DESC LIMIT 100")?;
        let rows = stmt.query_map([], |r| {
            Ok(Scan {
                time: r.get(0)?,
                name: r.get(1)?,
                outcome: r.get(2)?,
                simulation: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}
fn changed(n: usize) -> Result<(), Error> {
    if n == 0 {
        Err(Error::Missing)
    } else {
        Ok(())
    }
}
fn check_text(s: &str, required: bool) -> Result<(), Error> {
    if s.len() > 160 || s.chars().any(char::is_control) || (required && s.trim().is_empty()) {
        return Err(Error::Invalid("invalid label"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Vec<u8> {
        let mut b = vec![0; 540];
        b[0] = 4;
        b[3] = 0x8c;
        b[10..16].copy_from_slice(&[15, 224, 241, 16, 255, 238]);
        b[520..523].copy_from_slice(&[1, 0, 15]);
        b[527] = 4;
        b[528] = 95;
        b
    }
    #[test]
    fn structure_and_signature() {
        let mut b = sample();
        assert!(Tag::parse(&b).is_ok());
        b.resize(572, 1);
        assert!(Tag::parse(&b).is_ok());
        b[3] ^= 1;
        assert!(matches!(Tag::parse(&b), Err(Error::Invalid(_))));
        assert!(matches!(
            Tag::parse(&b[..520]),
            Err(Error::Unsupported(520))
        ));
    }
    #[test]
    fn selection_deletion_and_history() {
        let mut db = Library::open(":memory:").unwrap();
        let id = db.add("Test", &sample()).unwrap();
        assert!(matches!(
            db.add("duplicate", &sample()),
            Err(Error::Duplicate)
        ));
        db.select(id).unwrap();
        db.edit(id, "Test", true).unwrap();
        db.record("Test", "Transferred", true).unwrap();
        assert!(db.list().unwrap()[0].selected);
        db.delete(id).unwrap();
        assert!(matches!(db.selected(), Err(Error::Missing)));
        assert!(db.history().unwrap()[0].simulation);
    }
}
