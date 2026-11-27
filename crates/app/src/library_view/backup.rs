//! Settings → Backup: save settings, folders, collections and watch history to a JSON file,
//! and merge one back in (see `miru_core::backup`).

use miru_core::backup::{BACKUP_FORMAT, Backup};

use super::*;

impl LibraryView {
    fn set_backup_status(
        &mut self,
        text: impl Into<SharedString>,
        failed: bool,
        cx: &mut Context<Self>,
    ) {
        self.backup_status = Some((text.into(), failed));
        cx.notify();
    }

    /// Asks where to save, then writes the backup there.
    fn export_backup(&mut self, cx: &mut Context<Self>) {
        let dir = directories::UserDirs::new()
            .and_then(|d| d.document_dir().map(std::path::Path::to_path_buf))
            .or_else(|| directories::UserDirs::new().map(|d| d.home_dir().to_path_buf()))
            .unwrap_or_default();
        let now = unix_secs();
        let name = format!("Miru backup {}.json", date_label(now));
        let picked = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn(async move |this, cx| {
            let path = match picked.await {
                Ok(Ok(Some(path))) => path,
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(e)) => {
                    this.update(cx, |view, cx| {
                        view.set_backup_status(
                            format!("Couldn't open the save dialog: {e}"),
                            true,
                            cx,
                        )
                    })
                    .ok();
                    return;
                }
            };
            this.update(cx, |view, cx| {
                let written = view
                    .db
                    .export_backup(now)
                    .map_err(|e| e.to_string())
                    .and_then(|backup| {
                        serde_json::to_vec_pretty(&backup).map_err(|e| e.to_string())
                    })
                    .and_then(|json| std::fs::write(&path, json).map_err(|e| e.to_string()));
                match written {
                    Ok(()) => {
                        let file = path
                            .file_name()
                            .map(|f| f.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        view.set_backup_status(format!("Saved {file}."), false, cx);
                    }
                    Err(e) => {
                        view.set_backup_status(format!("Couldn't save the backup: {e}"), true, cx)
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Asks for a backup file and merges it into this library.
    fn import_backup(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Restore".into()),
        });
        cx.spawn(async move |this, cx| {
            let path = match picked.await {
                Ok(Ok(Some(paths))) => match paths.into_iter().next() {
                    Some(path) => path,
                    None => return,
                },
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(e)) => {
                    this.update(cx, |view, cx| {
                        view.set_backup_status(
                            format!("Couldn't open the file picker: {e}"),
                            true,
                            cx,
                        )
                    })
                    .ok();
                    return;
                }
            };
            this.update(cx, |view, cx| view.restore_from(&path, cx))
                .ok();
        })
        .detach();
    }

    fn restore_from(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        let backup: Backup =
            match std::fs::read(path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    serde_json::from_slice(&bytes).map_err(|_| "it isn't a Miru backup".to_string())
                }) {
                Ok(backup) => backup,
                Err(e) => {
                    self.set_backup_status(format!("Couldn't read that file: {e}."), true, cx);
                    return;
                }
            };
        if backup.app != "miru" {
            self.set_backup_status("That file isn't a Miru backup.", true, cx);
            return;
        }
        if backup.format > BACKUP_FORMAT {
            self.set_backup_status(
                "That backup is from a newer version of Miru. Update Miru, then try again.",
                true,
                cx,
            );
            return;
        }
        match self.db.restore_backup(&backup) {
            Ok(report) => {
                match self.db.settings() {
                    Ok(settings) => self.settings = settings,
                    Err(e) => tracing::error!("reading restored settings: {e}"),
                }
                self.refresh(cx);
                if report.folders_added > 0 {
                    self.start_sync(cx);
                }
                let mut parts = vec![format!(
                    "{} watched or in-progress item{}",
                    report.history_restored,
                    if report.history_restored == 1 {
                        ""
                    } else {
                        "s"
                    }
                )];
                if report.collections > 0 {
                    parts.push(format!(
                        "{} collection{}",
                        report.collections,
                        if report.collections == 1 { "" } else { "s" }
                    ));
                }
                if report.folders_added > 0 {
                    parts.push(format!(
                        "{} folder{}",
                        report.folders_added,
                        if report.folders_added == 1 { "" } else { "s" }
                    ));
                }
                let mut message = format!("Restored {} and your settings.", parts.join(", "));
                if report.history_unmatched > 0 {
                    message.push_str(&format!(
                        " {} item{} from the backup aren't in this library.",
                        report.history_unmatched,
                        if report.history_unmatched == 1 {
                            ""
                        } else {
                            "s"
                        }
                    ));
                }
                self.set_backup_status(message, false, cx);
            }
            Err(e) => self.set_backup_status(format!("Couldn't restore the backup: {e}"), true, cx),
        }
    }

    pub(super) fn backup_rows(&self, cx: &mut Context<Self>) -> Vec<SettingsRow> {
        let export = theme::compact_button(
            "backup-export",
            Some("icons/download.svg"),
            "Back Up…",
            cx.listener(|this, _, _, cx| this.export_backup(cx)),
        );
        let import = theme::compact_button(
            "backup-import",
            Some("icons/restart.svg"),
            "Restore…",
            cx.listener(|this, _, _, cx| this.import_backup(cx)),
        );
        let mut rows = vec![
            setting_row(
                "Back up",
                Some(
                    "Save your settings, library folders, collections and watch history to a file. \
                     Your TMDB key isn't included.",
                ),
                Some(export.into_any_element()),
            ),
            setting_row(
                "Restore",
                Some(
                    "Merge a backup into this library. Collections are combined, and watch history \
                     is matched to the videos already here.",
                ),
                Some(import.into_any_element()),
            ),
        ];
        if let Some((text, failed)) = &self.backup_status {
            rows.push(SettingsRow::plain(
                div()
                    .px_5()
                    .py_3()
                    .text_sm()
                    .text_color(if *failed {
                        rgba(0xff9b9bff)
                    } else {
                        theme::text_secondary()
                    })
                    .child(text.clone()),
            ));
        }
        rows
    }
}

fn unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// "2026-11-23" for a unix time (UTC), for the suggested file name.
fn date_label(secs: i64) -> String {
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_from_unix_time() {
        assert_eq!(date_label(0), "1970-01-01");
        assert_eq!(date_label(1_795_910_400), "2026-11-29");
        assert_eq!(date_label(951_782_400), "2000-02-29");
    }
}
