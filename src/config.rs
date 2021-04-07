//! # Key-value configuration management

use anyhow::Result;
use strum::{EnumProperty, IntoEnumIterator};
use strum_macros::{AsRefStr, Display, EnumIter, EnumProperty, EnumString};

use crate::blob::BlobObject;
use crate::chat::ChatId;
use crate::constants::DC_VERSION_STR;
use crate::context::Context;
use crate::dc_tools::{dc_get_abs_path, improve_single_line_input};
use crate::events::EventType;
use crate::message::MsgId;
use crate::mimefactory::RECOMMENDED_FILE_SIZE;
use crate::provider::{get_provider_by_id, Provider};
use crate::stock_str;

/// The available configuration keys.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Display, EnumString, AsRefStr, EnumIter, EnumProperty,
)]
#[strum(serialize_all = "snake_case")]
pub enum Config {
    Addr,
    MailServer,
    MailUser,
    MailPw,
    MailPort,
    MailSecurity,
    ImapCertificateChecks,
    SendServer,
    SendUser,
    SendPw,
    SendPort,
    SendSecurity,
    SmtpCertificateChecks,
    ServerFlags,

    Displayname,
    Selfstatus,
    Selfavatar,

    #[strum(props(default = "0"))]
    BccSelf,

    #[strum(props(default = "1"))]
    E2eeEnabled,

    #[strum(props(default = "1"))]
    MdnsEnabled,

    #[strum(props(default = "1"))]
    InboxWatch,

    #[strum(props(default = "1"))]
    SentboxWatch,

    #[strum(props(default = "1"))]
    MvboxWatch,

    #[strum(props(default = "1"))]
    MvboxMove,

    #[strum(props(default = "0"))]
    SentboxMove, // If `MvboxMove` is true, this config is ignored. Currently only used in tests.

    #[strum(props(default = "0"))] // also change ShowEmails.default() on changes
    ShowEmails,

    #[strum(props(default = "0"))] // also change MediaQuality.default() on changes
    MediaQuality,

    /// If set to "1", on the first time `start_io()` is called after configuring,
    /// the newest existing messages are fetched.
    /// Existing recipients are added to the contact database regardless of this setting.
    #[strum(props(default = "1"))]
    FetchExistingMsgs,

    #[strum(props(default = "0"))]
    KeyGenType,

    /// Timer in seconds after which the message is deleted from the
    /// server.
    ///
    /// Equals to 0 by default, which means the message is never
    /// deleted.
    ///
    /// Value 1 is treated as "delete at once": messages are deleted
    /// immediately, without moving to DeltaChat folder.
    #[strum(props(default = "0"))]
    DeleteServerAfter,

    /// Timer in seconds after which the message is deleted from the
    /// device.
    ///
    /// Equals to 0 by default, which means the message is never
    /// deleted.
    #[strum(props(default = "0"))]
    DeleteDeviceAfter,

    SaveMimeHeaders,
    ConfiguredAddr,
    ConfiguredMailServer,
    ConfiguredMailUser,
    ConfiguredMailPw,
    ConfiguredMailPort,
    ConfiguredMailSecurity,
    ConfiguredImapCertificateChecks,
    ConfiguredSendServer,
    ConfiguredSendUser,
    ConfiguredSendPw,
    ConfiguredSendPort,
    ConfiguredSmtpCertificateChecks,
    ConfiguredServerFlags,
    ConfiguredSendSecurity,
    ConfiguredE2EEEnabled,
    ConfiguredInboxFolder,
    ConfiguredMvboxFolder,
    ConfiguredSentboxFolder,
    ConfiguredSpamFolder,
    ConfiguredTimestamp,
    ConfiguredProvider,
    Configured,

    #[strum(serialize = "sys.version")]
    SysVersion,

    #[strum(serialize = "sys.msgsize_max_recommended")]
    SysMsgsizeMaxRecommended,

    #[strum(serialize = "sys.config_keys")]
    SysConfigKeys,

    Bot,

    /// Whether we send a warning if the password is wrong (set to false when we send a warning
    /// because we do not want to send a second warning)
    #[strum(props(default = "0"))]
    NotifyAboutWrongPw,

    /// address to webrtc instance to use for videochats
    WebrtcInstance,

    /// Timestamp of the last time housekeeping was run
    LastHousekeeping,

    /// To how many seconds to debounce scan_all_folders. Used mainly in tests, to disable debouncing completely.
    #[strum(props(default = "60"))]
    ScanAllFoldersDebounceSecs,
}

impl Context {
    pub async fn config_exists(&self, key: Config) -> Result<bool> {
        Ok(self.sql.get_raw_config(key).await?.is_some())
    }

    /// Get a configuration key. Returns `None` if no value is set, and no default value found.
    pub async fn get_config(&self, key: Config) -> Result<Option<String>> {
        let value = match key {
            Config::Selfavatar => {
                let rel_path = self.sql.get_raw_config(key).await?;
                rel_path.map(|p| dc_get_abs_path(self, &p).to_string_lossy().into_owned())
            }
            Config::SysVersion => Some((&*DC_VERSION_STR).clone()),
            Config::SysMsgsizeMaxRecommended => Some(format!("{}", RECOMMENDED_FILE_SIZE)),
            Config::SysConfigKeys => Some(get_config_keys_string()),
            _ => self.sql.get_raw_config(key).await?,
        };

        if value.is_some() {
            return Ok(value);
        }

        // Default values
        match key {
            Config::Selfstatus => Ok(Some(stock_str::status_line(self).await)),
            Config::ConfiguredInboxFolder => Ok(Some("INBOX".to_owned())),
            _ => Ok(key.get_str("default").map(|s| s.to_string())),
        }
    }

    pub async fn get_config_int(&self, key: Config) -> Result<i32> {
        self.get_config(key)
            .await
            .map(|s: Option<String>| s.and_then(|s| s.parse().ok()).unwrap_or_default())
    }

    pub async fn get_config_i64(&self, key: Config) -> Result<i64> {
        self.get_config(key)
            .await
            .map(|s: Option<String>| s.and_then(|s| s.parse().ok()).unwrap_or_default())
    }

    pub async fn get_config_u64(&self, key: Config) -> Result<u64> {
        self.get_config(key)
            .await
            .map(|s: Option<String>| s.and_then(|s| s.parse().ok()).unwrap_or_default())
    }

    pub async fn get_config_bool(&self, key: Config) -> Result<bool> {
        Ok(self.get_config_int(key).await? != 0)
    }

    /// Gets configured "delete_server_after" value.
    ///
    /// `None` means never delete the message, `Some(0)` means delete
    /// at once, `Some(x)` means delete after `x` seconds.
    pub async fn get_config_delete_server_after(&self) -> Result<Option<i64>> {
        match self.get_config_int(Config::DeleteServerAfter).await? {
            0 => Ok(None),
            1 => Ok(Some(0)),
            x => Ok(Some(x as i64)),
        }
    }

    /// Gets the configured provider, as saved in the `configured_provider` value.
    ///
    /// The provider is determined by `get_provider_info()` during configuration and then saved
    /// to the db in `param.save_to_database()`, together with all the other `configured_*` values.
    pub async fn get_configured_provider(&self) -> Result<Option<&'static Provider>> {
        if let Some(cfg) = self.get_config(Config::ConfiguredProvider).await? {
            return Ok(get_provider_by_id(&cfg));
        }
        Ok(None)
    }

    /// Gets configured "delete_device_after" value.
    ///
    /// `None` means never delete the message, `Some(x)` means delete
    /// after `x` seconds.
    pub async fn get_config_delete_device_after(&self) -> Result<Option<i64>> {
        match self.get_config_int(Config::DeleteDeviceAfter).await? {
            0 => Ok(None),
            x => Ok(Some(x as i64)),
        }
    }

    /// Set the given config key.
    /// If `None` is passed as a value the value is cleared and set to the default if there is one.
    pub async fn set_config(&self, key: Config, value: Option<&str>) -> Result<()> {
        match key {
            Config::Selfavatar => {
                self.sql
                    .execute("UPDATE contacts SET selfavatar_sent=0;")
                    .await?;
                self.sql
                    .set_raw_config_bool("attach_selfavatar", true)
                    .await?;
                match value {
                    Some(value) => {
                        let blob = BlobObject::new_from_path(self, value).await?;
                        blob.recode_to_avatar_size(self).await?;
                        self.sql.set_raw_config(key, Some(blob.as_name())).await?;
                        Ok(())
                    }
                    None => {
                        self.sql.set_raw_config(key, None).await?;
                        Ok(())
                    }
                }
            }
            Config::Selfstatus => {
                let def = stock_str::status_line(self).await;
                let val = if value.is_none() || value.unwrap() == def {
                    None
                } else {
                    value
                };

                self.sql.set_raw_config(key, val).await?;
                Ok(())
            }
            Config::DeleteDeviceAfter => {
                let ret = self
                    .sql
                    .set_raw_config(key, value)
                    .await
                    .map_err(Into::into);
                // Force chatlist reload to delete old messages immediately.
                self.emit_event(EventType::MsgsChanged {
                    msg_id: MsgId::new(0),
                    chat_id: ChatId::new(0),
                });
                ret
            }
            Config::Displayname => {
                let value = value.map(improve_single_line_input);
                self.sql.set_raw_config(key, value.as_deref()).await?;
                Ok(())
            }
            _ => {
                self.sql.set_raw_config(key, value).await?;
                Ok(())
            }
        }
    }

    pub async fn set_config_bool(&self, key: Config, value: bool) -> crate::sql::Result<()> {
        self.set_config(key, if value { Some("1") } else { None })
            .await?;
        Ok(())
    }
}

/// Returns all available configuration keys concated together.
fn get_config_keys_string() -> String {
    let keys = Config::iter().fold(String::new(), |mut acc, key| {
        acc += key.as_ref();
        acc += " ";
        acc
    });

    format!(" {} ", keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::str::FromStr;
    use std::string::ToString;

    use crate::constants;
    use crate::constants::BALANCED_AVATAR_SIZE;
    use crate::test_utils::TestContext;
    use image::GenericImageView;
    use num_traits::FromPrimitive;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_to_string() {
        assert_eq!(Config::MailServer.to_string(), "mail_server");
        assert_eq!(Config::from_str("mail_server"), Ok(Config::MailServer));

        assert_eq!(Config::SysConfigKeys.to_string(), "sys.config_keys");
        assert_eq!(
            Config::from_str("sys.config_keys"),
            Ok(Config::SysConfigKeys)
        );
    }

    #[async_std::test]
    async fn test_selfavatar_outside_blobdir() {
        let t = TestContext::new().await;
        let avatar_src = t.dir.path().join("avatar.jpg");
        let avatar_bytes = include_bytes!("../test-data/image/avatar1000x1000.jpg");
        File::create(&avatar_src)
            .unwrap()
            .write_all(avatar_bytes)
            .unwrap();
        let avatar_blob = t.get_blobdir().join("avatar.jpg");
        assert!(!avatar_blob.exists().await);
        t.set_config(Config::Selfavatar, Some(avatar_src.to_str().unwrap()))
            .await
            .unwrap();
        assert!(avatar_blob.exists().await);
        assert!(std::fs::metadata(&avatar_blob).unwrap().len() < avatar_bytes.len() as u64);
        let avatar_cfg = t.get_config(Config::Selfavatar).await.unwrap();
        assert_eq!(avatar_cfg, avatar_blob.to_str().map(|s| s.to_string()));

        let img = image::open(avatar_src).unwrap();
        assert_eq!(img.width(), 1000);
        assert_eq!(img.height(), 1000);

        let img = image::open(avatar_blob).unwrap();
        assert_eq!(img.width(), BALANCED_AVATAR_SIZE);
        assert_eq!(img.height(), BALANCED_AVATAR_SIZE);
    }

    #[async_std::test]
    async fn test_selfavatar_in_blobdir() {
        let t = TestContext::new().await;
        let avatar_src = t.get_blobdir().join("avatar.png");
        let avatar_bytes = include_bytes!("../test-data/image/avatar900x900.png");
        File::create(&avatar_src)
            .unwrap()
            .write_all(avatar_bytes)
            .unwrap();

        let img = image::open(&avatar_src).unwrap();
        assert_eq!(img.width(), 900);
        assert_eq!(img.height(), 900);

        t.set_config(Config::Selfavatar, Some(avatar_src.to_str().unwrap()))
            .await
            .unwrap();
        let avatar_cfg = t.get_config(Config::Selfavatar).await.unwrap();
        assert_eq!(avatar_cfg, avatar_src.to_str().map(|s| s.to_string()));

        let img = image::open(avatar_src).unwrap();
        assert_eq!(img.width(), BALANCED_AVATAR_SIZE);
        assert_eq!(img.height(), BALANCED_AVATAR_SIZE);
    }

    #[async_std::test]
    async fn test_selfavatar_copy_without_recode() {
        let t = TestContext::new().await;
        let avatar_src = t.dir.path().join("avatar.png");
        let avatar_bytes = include_bytes!("../test-data/image/avatar64x64.png");
        File::create(&avatar_src)
            .unwrap()
            .write_all(avatar_bytes)
            .unwrap();
        let avatar_blob = t.get_blobdir().join("avatar.png");
        assert!(!avatar_blob.exists().await);
        t.set_config(Config::Selfavatar, Some(avatar_src.to_str().unwrap()))
            .await
            .unwrap();
        assert!(avatar_blob.exists().await);
        assert_eq!(
            std::fs::metadata(&avatar_blob).unwrap().len(),
            avatar_bytes.len() as u64
        );
        let avatar_cfg = t.get_config(Config::Selfavatar).await.unwrap();
        assert_eq!(avatar_cfg, avatar_blob.to_str().map(|s| s.to_string()));
    }

    #[async_std::test]
    async fn test_media_quality_config_option() {
        let t = TestContext::new().await;
        let media_quality = t.get_config_int(Config::MediaQuality).await.unwrap();
        assert_eq!(media_quality, 0);
        let media_quality = constants::MediaQuality::from_i32(media_quality).unwrap_or_default();
        assert_eq!(media_quality, constants::MediaQuality::Balanced);

        t.set_config(Config::MediaQuality, Some("1")).await.unwrap();

        let media_quality = t.get_config_int(Config::MediaQuality).await.unwrap();
        assert_eq!(media_quality, 1);
        assert_eq!(constants::MediaQuality::Worse as i32, 1);
        let media_quality = constants::MediaQuality::from_i32(media_quality).unwrap_or_default();
        assert_eq!(media_quality, constants::MediaQuality::Worse);
    }
}
