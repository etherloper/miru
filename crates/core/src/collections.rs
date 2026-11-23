//! Collections: named lists of movies and shows the user puts together (e.g. "Comfort
//! rewatches"), shown in the sidebar.

use rusqlite::{OptionalExtension, params};

use crate::db::{Db, Result};
use crate::library::{ItemKind, TitleKey};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collection {
    pub id: i64,
    pub name: String,
    pub item_count: u32,
}

impl Db {
    /// Every collection, in the order the user arranged them.
    pub fn collections(&self) -> Result<Vec<Collection>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.name, COUNT(i.collection_id)
             FROM collections c LEFT JOIN collection_items i ON i.collection_id = c.id
             GROUP BY c.id ORDER BY c.position, c.id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Collection {
                id: r.get(0)?,
                name: r.get(1)?,
                item_count: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Adds an empty collection at the end and returns its id.
    pub fn create_collection(&self, name: &str) -> Result<i64> {
        let position: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM collections",
            [],
            |r| r.get(0),
        )?;
        self.conn.execute(
            "INSERT INTO collections (name, position, created_at)
             VALUES (?1, ?2, strftime('%s', 'now'))",
            params![name, position],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn rename_collection(&self, id: i64, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE collections SET name = ?2 WHERE id = ?1",
            params![id, name],
        )?;
        Ok(())
    }

    /// Deletes a collection (its titles stay in the library).
    pub fn delete_collection(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM collections WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Adds a title to a collection. Returns false if it was already there.
    pub fn add_to_collection(&self, id: i64, key: &TitleKey) -> Result<bool> {
        let added = self.conn.execute(
            "INSERT OR IGNORE INTO collection_items (collection_id, kind, title, year, added_at)
             VALUES (?1, ?2, ?3, ?4, strftime('%s', 'now'))",
            params![id, key.kind.as_str(), key.title, key.year.unwrap_or(0)],
        )?;
        Ok(added > 0)
    }

    pub fn remove_from_collection(&self, id: i64, key: &TitleKey) -> Result<()> {
        self.conn.execute(
            "DELETE FROM collection_items
             WHERE collection_id = ?1 AND kind = ?2 AND title = ?3 AND year = ?4",
            params![id, key.kind.as_str(), key.title, key.year.unwrap_or(0)],
        )?;
        Ok(())
    }

    /// The titles in a collection, most recently added first.
    pub fn collection_keys(&self, id: i64) -> Result<Vec<TitleKey>> {
        let mut stmt = self.conn.prepare(
            "SELECT kind, title, year FROM collection_items
             WHERE collection_id = ?1 ORDER BY added_at DESC, rowid DESC",
        )?;
        let rows = stmt.query_map([id], |r| {
            let kind: String = r.get(0)?;
            let year: i64 = r.get(2)?;
            Ok(TitleKey {
                kind: if kind == "show" {
                    ItemKind::Show
                } else {
                    ItemKind::Movie
                },
                title: r.get(1)?,
                year: u16::try_from(year).ok().filter(|y| *y > 0),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Ids of the collections a title is in.
    pub fn collections_with(&self, key: &TitleKey) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT collection_id FROM collection_items
             WHERE kind = ?1 AND title = ?2 AND year = ?3",
        )?;
        let rows = stmt.query_map(
            params![key.kind.as_str(), key.title, key.year.unwrap_or(0)],
            |r| r.get(0),
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Moves a collection to `index` in the sidebar order.
    pub fn move_collection(&self, id: i64, index: usize) -> Result<()> {
        let mut ids: Vec<i64> = self.collections()?.into_iter().map(|c| c.id).collect();
        let Some(from) = ids.iter().position(|c| *c == id) else {
            return Ok(());
        };
        let moved = ids.remove(from);
        ids.insert(index.min(ids.len()), moved);
        for (position, id) in ids.iter().enumerate() {
            self.conn.execute(
                "UPDATE collections SET position = ?2 WHERE id = ?1",
                params![id, position as i64],
            )?;
        }
        Ok(())
    }

    pub fn collection_name(&self, id: i64) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT name FROM collections WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(kind: ItemKind, title: &str, year: Option<u16>) -> TitleKey {
        TitleKey {
            kind,
            title: title.into(),
            year,
        }
    }

    #[test]
    fn collections_hold_titles() {
        let db = Db::open_in_memory().unwrap();
        let faves = db.create_collection("Favourites").unwrap();
        let later = db.create_collection("Later").unwrap();
        let heat = key(ItemKind::Movie, "Heat", Some(1995));
        let dark = key(ItemKind::Show, "Dark", None);

        assert!(db.add_to_collection(faves, &heat).unwrap());
        assert!(
            !db.add_to_collection(faves, &heat).unwrap(),
            "no duplicates"
        );
        assert!(db.add_to_collection(faves, &dark).unwrap());
        assert!(db.add_to_collection(later, &dark).unwrap());

        let keys = db.collection_keys(faves).unwrap();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&heat) && keys.contains(&dark));
        assert_eq!(db.collections_with(&dark).unwrap().len(), 2);

        let all = db.collections().unwrap();
        assert_eq!(
            all.iter()
                .map(|c| (c.name.as_str(), c.item_count))
                .collect::<Vec<_>>(),
            [("Favourites", 2), ("Later", 1)]
        );

        db.move_collection(later, 0).unwrap();
        db.rename_collection(faves, "Best").unwrap();
        let names: Vec<String> = db
            .collections()
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["Later", "Best"]);

        db.remove_from_collection(faves, &heat).unwrap();
        assert_eq!(
            db.collection_keys(faves).unwrap(),
            std::slice::from_ref(&dark)
        );
        db.delete_collection(faves).unwrap();
        assert_eq!(db.collections_with(&dark).unwrap(), [later]);
    }
}
