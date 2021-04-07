use std::time::Instant;

use anyhow::{Context as _, Result};

use crate::config::Config;
use crate::context::Context;
use crate::imap::Imap;
use crate::log::LogExt;

use async_std::stream::StreamExt;

use super::{get_folder_meaning, get_folder_meaning_by_name, FolderMeaning};

impl Imap {
    pub async fn scan_folders(&mut self, context: &Context) -> Result<()> {
        // First of all, debounce to once per minute:
        let mut last_scan = context.last_full_folder_scan.lock().await;
        if let Some(last_scan) = *last_scan {
            let elapsed_secs = last_scan.elapsed().as_secs();
            let debounce_secs = context
                .get_config_u64(Config::ScanAllFoldersDebounceSecs)
                .await?;

            if elapsed_secs < debounce_secs {
                return Ok(());
            }
        }
        info!(context, "Starting full folder scan");

        self.setup_handle(context).await?;
        let session = self.session.as_mut();
        let session = session.context("scan_folders(): IMAP No Connection established")?;
        let folders: Vec<_> = session.list(Some(""), Some("*")).await?.collect().await;
        let watched_folders = get_watched_folders(context).await;

        let mut sentbox_folder = None;
        let mut spam_folder = None;

        for folder in folders {
            let folder = match folder {
                Ok(f) => f,
                Err(e) => {
                    warn!(context, "Can't get folder: {}", e);
                    continue;
                }
            };

            let foldername = folder.name();
            let folder_meaning = get_folder_meaning(&folder);
            let folder_name_meaning = get_folder_meaning_by_name(foldername);

            if folder_meaning == FolderMeaning::SentObjects {
                // Always takes precedent
                sentbox_folder = Some(folder.name().to_string());
            } else if folder_meaning == FolderMeaning::Spam {
                spam_folder = Some(folder.name().to_string());
            } else if folder_name_meaning == FolderMeaning::SentObjects {
                // only set iff none has been already set
                if sentbox_folder.is_none() {
                    sentbox_folder = Some(folder.name().to_string());
                }
            } else if folder_name_meaning == FolderMeaning::Spam && spam_folder.is_none() {
                spam_folder = Some(folder.name().to_string());
            }

            if watched_folders.contains(&foldername.to_string()) {
                info!(
                    context,
                    "Not scanning folder {} as it is watched anyway", foldername
                );
            } else {
                info!(context, "Scanning folder: {}", foldername);

                if let Err(e) = self.fetch_new_messages(context, foldername, false).await {
                    warn!(context, "Can't fetch new msgs in scanned folder: {:#}", e);
                }
            }
        }

        context
            .set_config(Config::ConfiguredSentboxFolder, sentbox_folder.as_deref())
            .await?;
        context
            .set_config(Config::ConfiguredSpamFolder, spam_folder.as_deref())
            .await?;

        last_scan.replace(Instant::now());
        Ok(())
    }

    /// Returns the names of all folders on the IMAP server that are not in `exclude`.
    pub async fn list_folders_except(
        self: &mut Imap,
        context: &Context,
        exclude: &[impl AsRef<str> + Sync],
    ) -> Result<Vec<String>> {
        let session = self.session.as_mut();
        let session = session.context("list_folders_except() IMAP No Connection established")?;
        let list = session
            .list(Some(""), Some("*"))
            .await?
            .filter_map(|f| f.ok_or_log_msg(context, "list_folders_except can't get folder"))
            .map(|f| f.name().to_string())
            .filter(|name| !exclude.iter().any(|s| s.as_ref() == name));
        Ok(list.collect().await)
    }
}

async fn get_watched_folders(context: &Context) -> Vec<String> {
    let mut res = Vec::new();
    let folder_watched_configured = &[
        (Config::SentboxWatch, Config::ConfiguredSentboxFolder),
        (Config::MvboxWatch, Config::ConfiguredMvboxFolder),
        (Config::InboxWatch, Config::ConfiguredInboxFolder),
    ];
    for (watched, configured) in folder_watched_configured {
        if context.get_config_bool(*watched).await.unwrap_or_default() {
            if let Ok(Some(folder)) = context.get_config(*configured).await {
                res.push(folder);
            }
        }
    }
    res
}
