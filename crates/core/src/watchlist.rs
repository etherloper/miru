//! The watchlist: titles saved to watch later, shown on Home. Keyed like collections (kind,
//! title, year with 0 for none), so entries survive rescans.

use rusqlite::params;

use crate::db::{Db, Result};
use crate::library::{ItemKind, TitleKey};

impl Db {
    /// Watchlist titles, most recently added first.
    pub fn watchlist(&self) -> Result<Vec<TitleKey>> {
        let mut stmt = self.conn.prepare(
            "SELECT kind, title, year FROM watchlist ORDER BY added_at DESC, rowid DESC",
        )?;
        let rows = stmt.query_map([], |r| {
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

    /// Adds a title to the watchlist or takes it off.
    pub fn set_on_watchlist(&self, key: &TitleKey, on: bool) -> Result<()> {
        let args = params![key.kind.as_str(), key.title, key.year.unwrap_or(0)];
        if on {
            self.conn.execute(
                "INSERT OR IGNORE INTO watchlist (kind, title, year, added_at)
                 VALUES (?1, ?2, ?3, strftime('%s', 'now'))",
                args,
            )?;
        } else {
            self.conn.execute(
                "DELETE FROM watchlist WHERE kind = ?1 AND title = ?2 AND year = ?3",
                args,
            )?;
        }
        Ok(())
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
    fn adds_lists_and_removes() {
        let db = Db::open_in_memory().unwrap();
        let heat = key(ItemKind::Movie, "Heat", Some(1995));
        let dark = key(ItemKind::Show, "Dark", None);
        db.set_on_watchlist(&heat, true).unwrap();
        db.set_on_watchlist(&dark, true).unwrap();
        // Adding twice keeps one entry.
        db.set_on_watchlist(&dark, true).unwrap();
        let list = db.watchlist().unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.contains(&heat) && list.contains(&dark));

        // Titles match without regard to case, like the rest of the library.
        db.set_on_watchlist(&key(ItemKind::Movie, "heat", Some(1995)), false)
            .unwrap();
        assert_eq!(db.watchlist().unwrap(), vec![dark]);
    }
}
