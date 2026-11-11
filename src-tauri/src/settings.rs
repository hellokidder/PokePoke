use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// popup 的规格尺寸:默认(标准大尺寸) / 迷你(紧凑条状)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PopupSize {
    #[default]
    Default,
    Mini,
}

impl PopupSize {
    /// kebab-case string,与 URL hash 参数 `size=` 匹配。
    pub fn as_kebab_str(self) -> &'static str {
        match self {
            PopupSize::Default => "default",
            PopupSize::Mini => "mini",
        }
    }
}

/// popup 出现的屏幕角落。stack 方向跟随上 / 下:top 向下堆叠,bottom 向上堆叠。
/// CSS slide-in 动画方向跟随左 / 右:right 从右滑入,left 从左滑入。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PopupPosition {
    TopLeft,
    #[default]
    TopRight,
    BottomLeft,
    BottomRight,
}

impl PopupPosition {
    /// URL query 参数 / CSS class 后缀,与前端 PopupWindow `?position=` 匹配。
    pub fn as_kebab_str(self) -> &'static str {
        match self {
            PopupPosition::TopLeft => "top-left",
            PopupPosition::TopRight => "top-right",
            PopupPosition::BottomLeft => "bottom-left",
            PopupPosition::BottomRight => "bottom-right",
        }
    }
}

/// 多屏环境下 popup 显示在哪一块屏。
/// - `Terminal`(默认):popup 弹在触发事件的终端窗口所在屏 — 用 AppleScript
///   按 session.terminal_tty 查 iTerm2 / Terminal 窗口 bounds,中心点找包含的 monitor。
///   失败 (无 tty / Cursor GUI / 终端已关) 时 popup.rs 自己 fallback 到鼠标屏。
/// - `Primary`:popup 始终在主屏,无论终端 / 鼠标在哪 — 适合习惯通知统一
///   集中在主屏的用户。
///
/// `#[serde(alias = "active-cursor")]` 兼容老版本曾经的 ActiveCursor 取值,
/// 升级后老 settings.json 自动当 Terminal 使用(语义最接近)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PopupScreenStrategy {
    #[default]
    #[serde(alias = "active-cursor")]
    Terminal,
    Primary,
}

/// 菜单栏托盘图标样式。
/// - `Colored`:现有彩色八爪鱼(prod 紫 / dev 绿 hue +120°)。
/// - `White`:固定白色像素图,加载时不走 macOS template image 路径
///   (`icon_as_template(false)`),所以系统**不**按菜单栏背景反色——
///   深色菜单栏正常显示白色,浅色菜单栏会几乎隐形(用户的明确选择)。
///   不区分 dev/prod。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TrayIconStyle {
    #[default]
    Colored,
    White,
}

/// 界面明暗主题。
/// - `System`(默认):跟随 macOS 系统外观
/// - `Light` / `Dark`:强制亮色 / 暗色
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum UiTheme {
    Light,
    Dark,
    #[default]
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// pending 提醒提示音:"builtin:waiting" / "system:Ping" / "mute" 等。
    #[serde(default = "default_sound_pending")]
    pub sound_pending: String,

    /// idle(一轮正常完成)提醒提示音。
    #[serde(default = "default_sound_idle")]
    pub sound_idle: String,

    /// last_failed 提醒提示音。
    #[serde(default = "default_sound_failed")]
    pub sound_failed: String,

    /// UI locale: "zh" or "en"
    #[serde(default = "default_locale")]
    pub locale: String,

    /// 界面明暗主题,默认跟随系统。
    #[serde(default)]
    pub ui_theme: UiTheme,

    /// 全局提示开关:控制 session 状态变化时是否弹 popup + 播放提示音。
    /// 关闭时 panel / tray 仍正常工作,只屏蔽"主动提醒"副作用。
    /// 默认 true,保持原有行为不变。
    #[serde(default = "default_notifications_enabled")]
    pub notifications_enabled: bool,

    /// popup 出现的屏幕角落,默认右上(原行为)。
    #[serde(default)]
    pub popup_position: PopupPosition,

    /// 多屏 popup 显示策略,默认跟随鼠标当前屏。
    #[serde(default)]
    pub popup_screen_strategy: PopupScreenStrategy,

    /// 菜单栏图标样式,默认彩色(原行为)。
    #[serde(default)]
    pub tray_icon_style: TrayIconStyle,

    /// popup 规格尺寸:Default(标准大尺寸) 或 Mini(紧凑条状)。
    #[serde(default)]
    pub popup_size: PopupSize,

    /// popup 自动关闭时长,单位秒。`None` = 永不自动关闭(默认,保持原行为,
    /// popup 只在用户聚焦终端或手动点掉时消失)。`Some(n)` = popup 显示 n 秒
    /// 后自动关闭。前端目前只暴露 5 / 15 / 30 / 60 / 180 五档,但后端不做
    /// 白名单校验 —— 任何 >0 的 u32 都会被尊重,方便后续扩档或用户手改 JSON。
    #[serde(default)]
    pub popup_auto_close_secs: Option<u32>,

    /// 后台自动检查 PokePoke 自身更新。`true`(默认)= 启动 5s 后首检、之后
    /// 每 24h 一次,命中新版且未跳过则弹"升级 popup"。`false` = 关掉后台
    /// 巡检,只剩设置面板的「检查更新」手动按钮可用。
    #[serde(default = "default_update_auto_check")]
    pub update_auto_check: bool,

    /// 用户对某个版本号显式点过「跳过本版」。同版本号巡检命中时静默,
    /// 直到出现更高版本号才会再弹。手动按钮路径**不**受此约束,用户主动
    /// 点检查时仍会显示该版本(避免"我自己想再装一次但被自己跳过卡住")。
    #[serde(default)]
    pub update_skip_version: Option<String>,

    /// 聚焦抑制:聚焦在终端会话时,仅播放提示音、不弹出提示窗。
    /// 默认 true(保持原有行为)。关闭后终端聚焦时也会弹 popup。
    #[serde(default = "default_true")]
    pub focus_suppression_enabled: bool,

    /// popup 全局快捷键的前缀修饰键(不含数字主键)。格式与历史 panel_shortcut
    /// 一致,如 `CmdOrCtrl` / `Control+Shift`;数字键固定为 1~9,⌘Enter 仍是
    /// 第 1 个 popup 的别名。默认 `CmdOrCtrl`(macOS 上即 ⌘)。
    #[serde(default = "default_popup_shortcut_modifiers")]
    pub popup_shortcut_modifiers: String,

    /// 静谧模式全局快捷键(完整组合:modifiers + key)。与 popup 前缀**解耦**,
    /// 用户可独立配置。格式如 `CmdOrCtrl+Semicolon` / `Control+Shift+KeyK`,
    /// 末尾 token 必须是一个 KeyboardEvent `code`(会传给 `tauri_plugin_global_shortcut`
    /// 的 `Code` 解析),前面的 token 是 modifiers。默认 `CmdOrCtrl+Semicolon`
    /// —— 换掉了 v1.1.1 里默认 `⌘Z` 劫持全局 Undo 的行为。
    #[serde(default = "default_quiet_mode_shortcut")]
    pub quiet_mode_shortcut: String,

    /// 静谧模式默认倒计时时长(分钟)。picker 记住上次选择。
    #[serde(default = "default_quiet_mode_duration_mins")]
    pub quiet_mode_duration_mins: u32,

    /// 静谧模式 picker 最后拖拽位置(x,y),None=居中。
    /// 拖拽结束后写回 settings;打开时优先读此值。
    #[serde(default)]
    pub quiet_picker_last_position: Option<(f64, f64)>,

    /// 隐性补齐的接入端集合(如 cc / codex / cursor / opencode)。
    /// 首次启动 check 发现某端未接时静默 install 并把 source 记在这里;
    /// 用户手动 disconnect 不删此项,后续重启该端就不会被隐性装回来。
    #[serde(default)]
    pub auto_connected_sources: Vec<String>,

    /// 用户主动断开的接入端(cc / codex / cursor / opencode)。
    /// `/notify` 处理前检查:在列表内则忽略事件;断开时同步清理该端 session。
    #[serde(default)]
    pub disconnected_sources: Vec<String>,
}

fn default_true() -> bool {
    true
}

fn default_notifications_enabled() -> bool {
    true
}

fn default_update_auto_check() -> bool {
    true
}

fn default_sound_pending() -> String {
    "builtin:waiting".into()
}

fn default_sound_idle() -> String {
    "builtin:completed".into()
}

fn default_sound_failed() -> String {
    "builtin:failed".into()
}

fn default_popup_shortcut_modifiers() -> String {
    "CmdOrCtrl".into()
}

pub(crate) fn default_quiet_mode_shortcut() -> String {
    "CmdOrCtrl+Semicolon".into()
}

fn default_quiet_mode_duration_mins() -> u32 {
    30
}

/// 把语言标签(如 "zh-Hans-CN" / "en-US")归一到产品支持的两种 locale。
/// 以 "zh" 开头 → 中文(涵盖 zh-CN/zh-TW/zh-Hant 等),其余一律英文;取不到 → 中文兜底。
fn locale_from_tag(tag: Option<&str>) -> String {
    match tag {
        Some(t) if t.to_ascii_lowercase().starts_with("zh") => "zh".into(),
        Some(_) => "en".into(),
        None => "zh".into(),
    }
}

fn default_locale() -> String {
    locale_from_tag(sys_locale::get_locale().as_deref())
}

impl Settings {
    /// 按 session 提醒状态取对应提示音配置。Running 不应触发提醒,兜底 idle 音。
    pub fn sound_for_status(&self, status: crate::sessions::SessionStatus) -> &str {
        use crate::sessions::SessionStatus;
        match status {
            SessionStatus::Pending => &self.sound_pending,
            SessionStatus::Idle | SessionStatus::Running => &self.sound_idle,
            SessionStatus::LastFailed => &self.sound_failed,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sound_pending: default_sound_pending(),
            sound_idle: default_sound_idle(),
            sound_failed: default_sound_failed(),
            locale: default_locale(),
            ui_theme: UiTheme::default(),
            notifications_enabled: true,
            popup_position: PopupPosition::default(),
            popup_screen_strategy: PopupScreenStrategy::default(),
            tray_icon_style: TrayIconStyle::default(),
            popup_auto_close_secs: None,
            update_auto_check: true,
            update_skip_version: None,
            popup_size: PopupSize::default(),
            focus_suppression_enabled: true,
            popup_shortcut_modifiers: default_popup_shortcut_modifiers(),
            quiet_mode_shortcut: default_quiet_mode_shortcut(),
            quiet_mode_duration_mins: default_quiet_mode_duration_mins(),
            quiet_picker_last_position: None,
            auto_connected_sources: Vec::new(),
            disconnected_sources: Vec::new(),
        }
    }
}

pub struct SettingsStore {
    pub settings: Settings,
    file_path: PathBuf,
}

impl SettingsStore {
    pub fn load(file_path: PathBuf) -> Self {
        let mut migrated = false;
        let settings = if file_path.exists() {
            // 升级迁移:老 settings.json 可能因 umask 残留 group/other 可读位。
            // 读之前顺手收紧一次,跟 save() 对齐。失败不阻断 load。
            let _ = tighten_file_perms_0600(&file_path);
            let raw = fs::read_to_string(&file_path).ok();
            let mut settings = raw
                .as_ref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default();
            if let Some(s) = raw.as_ref() {
                if let Ok(json) = serde_json::from_str(s) {
                    migrated = migrate_legacy_alert_sound(&mut settings, &json);
                }
            }
            settings
        } else {
            Settings::default()
        };
        let store = Self {
            settings,
            file_path,
        };
        if migrated {
            // 一次性迁移落盘:磁盘 schema 收敛到新结构,避免每次 load 重复迁移,
            // 且任何"读原始 JSON 感知版本"的逻辑看到混合结构。
            store.save();
        }
        store
    }

    pub fn save(&self) {
        if let Some(parent) = self.file_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(
            &self.file_path,
            serde_json::to_string_pretty(&self.settings).unwrap_or_default(),
        );
        // 0600:settings.json 可能含飞书 app_secret / open_id 等凭证(移动端推送),
        // 落盘后必须只属主可读。对齐 license/state.rs 的 tighten_file_perms_0600;
        // 不能只靠 ~/.pokepoke/ 0700 + umask 兜底。失败不致命(沿用旧权限),仅静默。
        let _ = tighten_file_perms_0600(&self.file_path);
    }

    pub fn update(&mut self, new_settings: Settings) {
        self.settings = new_settings;
        self.save();
    }
}

/// 把已存在文件权限收紧到 0600(仅 unix;非 unix 无权限位概念,no-op)。
/// 复用思路跟 `license::state::tighten_file_perms_0600` / `transport::chmod_0600`
/// 一致;为避免 settings 反向依赖 license/transport 模块,各自独立维护一份。
/// 失败上抛,让调用方决定是否致命(save/load 都 best-effort 静默)。
#[cfg(unix)]
fn tighten_file_perms_0600(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn tighten_file_perms_0600(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

/// 老配置只有 `alert_sound` 单字段时,把该值复制到三种场景,避免升级后用户自定义音效丢失。
/// 返回是否真的发生了迁移(调用方据此决定是否落盘一次)。
fn migrate_legacy_alert_sound(settings: &mut Settings, json: &serde_json::Value) -> bool {
    let has_per_status = json.get("sound_pending").is_some()
        || json.get("sound_idle").is_some()
        || json.get("sound_failed").is_some();
    if has_per_status {
        return false;
    }
    if let Some(legacy) = json.get("alert_sound").and_then(|v| v.as_str()) {
        settings.sound_pending = legacy.to_string();
        settings.sound_idle = legacy.to_string();
        settings.sound_failed = legacy.to_string();
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{
        default_locale, locale_from_tag, PopupPosition, PopupScreenStrategy, PopupSize, Settings,
        SettingsStore, TrayIconStyle, UiTheme,
    };
    use std::path::PathBuf;
    use tempfile::{tempdir, TempDir};

    fn temp_store() -> (TempDir, SettingsStore, PathBuf) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let store = SettingsStore::load(path.clone());
        (dir, store, path)
    }

    #[test]
    fn load_returns_defaults_when_file_is_missing() {
        let (_dir, store, _path) = temp_store();

        assert_eq!(store.settings.sound_pending, "builtin:waiting");
        assert_eq!(store.settings.sound_idle, "builtin:completed");
        assert_eq!(store.settings.sound_failed, "builtin:failed");
        // locale 首启按系统语言探测,不再写死 zh;断言与探测结果一致,避免非中文 CI 机器误报。
        assert_eq!(
            store.settings.locale,
            locale_from_tag(sys_locale::get_locale().as_deref())
        );
        assert!(store.settings.notifications_enabled);
    }

    #[test]
    fn locale_from_tag_maps_language_tags() {
        assert_eq!(locale_from_tag(Some("zh-Hans-CN")), "zh");
        assert_eq!(locale_from_tag(Some("zh")), "zh");
        assert_eq!(locale_from_tag(Some("zh-TW")), "zh");
        assert_eq!(locale_from_tag(Some("en-US")), "en");
        assert_eq!(locale_from_tag(Some("ja-JP")), "en");
        assert_eq!(locale_from_tag(Some("de")), "en");
        assert_eq!(locale_from_tag(None), "zh");
    }

    #[test]
    fn update_and_reload_round_trip_preserves_settings() {
        let (_dir, mut store, path) = temp_store();
        let next = Settings {
            sound_pending: "mute".into(),
            sound_idle: "system:Ping".into(),
            sound_failed: "system:Basso".into(),
            locale: "en".into(),
            notifications_enabled: false,
            popup_position: PopupPosition::BottomLeft,
            popup_screen_strategy: PopupScreenStrategy::Primary,
            tray_icon_style: TrayIconStyle::White,
            popup_auto_close_secs: Some(30),
            popup_size: PopupSize::Mini,
            update_auto_check: false,
            update_skip_version: Some("0.0.7".into()),
            focus_suppression_enabled: false,
            popup_shortcut_modifiers: "Control".into(),
            quiet_mode_shortcut: "Control+Shift+KeyK".into(),
            quiet_mode_duration_mins: 15,
            quiet_picker_last_position: None,
            ui_theme: UiTheme::Dark,
            auto_connected_sources: Vec::new(),
            disconnected_sources: Vec::new(),
        };

        store.update(next);

        let reloaded = SettingsStore::load(path);
        assert_eq!(reloaded.settings.sound_pending, "mute");
        assert_eq!(reloaded.settings.sound_idle, "system:Ping");
        assert_eq!(reloaded.settings.sound_failed, "system:Basso");
        assert_eq!(reloaded.settings.locale, "en");
        assert!(!reloaded.settings.notifications_enabled);
        assert_eq!(reloaded.settings.popup_position, PopupPosition::BottomLeft);
        assert_eq!(
            reloaded.settings.popup_screen_strategy,
            PopupScreenStrategy::Primary,
        );
        assert_eq!(reloaded.settings.tray_icon_style, TrayIconStyle::White);
        assert_eq!(reloaded.settings.popup_auto_close_secs, Some(30));
        assert_eq!(reloaded.settings.popup_size, PopupSize::Mini);
        assert!(!reloaded.settings.update_auto_check);
        assert_eq!(
            reloaded.settings.update_skip_version.as_deref(),
            Some("0.0.7")
        );
        assert_eq!(reloaded.settings.ui_theme, UiTheme::Dark);
        assert_eq!(reloaded.settings.popup_shortcut_modifiers, "Control");
        assert_eq!(reloaded.settings.quiet_mode_shortcut, "Control+Shift+KeyK");
    }

    #[test]
    fn popup_shortcut_modifiers_defaults_to_cmd_or_ctrl_when_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"locale":"en"}"#).unwrap();
        let store = SettingsStore::load(path);
        assert_eq!(store.settings.popup_shortcut_modifiers, "CmdOrCtrl");
    }

    #[test]
    fn quiet_mode_shortcut_defaults_when_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"locale":"en"}"#).unwrap();
        let store = SettingsStore::load(path);
        assert_eq!(store.settings.quiet_mode_shortcut, "CmdOrCtrl+Semicolon");
    }

    #[test]
    fn ui_theme_defaults_to_system_when_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"locale":"en"}"#).unwrap();
        let store = SettingsStore::load(path);
        assert_eq!(store.settings.ui_theme, UiTheme::System);
    }

    #[test]
    fn popup_auto_close_secs_defaults_to_none() {
        // 老 settings.json 没有 popup_auto_close_secs 字段 → 默认 None(永不自动关闭)
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"locale":"en"}"#).unwrap();
        let store = SettingsStore::load(path);
        assert_eq!(store.settings.popup_auto_close_secs, None);
    }

    #[test]
    fn update_auto_check_defaults_to_true_when_missing() {
        // 0.0.2 升 0.0.3 老 settings.json 没字段 → 默认开自动巡检,跟"默认更安心"语义一致
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"locale":"en"}"#).unwrap();
        let store = SettingsStore::load(path);
        assert!(store.settings.update_auto_check);
        assert_eq!(store.settings.update_skip_version, None);
    }

    #[test]
    fn popup_screen_strategy_defaults_to_terminal() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"locale":"en"}"#).unwrap();
        let store = SettingsStore::load(path);
        assert_eq!(
            store.settings.popup_screen_strategy,
            PopupScreenStrategy::Terminal,
        );
    }

    #[test]
    fn popup_screen_strategy_legacy_active_cursor_alias_to_terminal() {
        // 老 settings.json 里曾经存的 "active-cursor" 应该自动当 Terminal 解读,
        // 升级零成本(serde alias 配置在 enum 上)。
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"locale":"en","popup_screen_strategy":"active-cursor"}"#,
        )
        .unwrap();
        let store = SettingsStore::load(path);
        assert_eq!(
            store.settings.popup_screen_strategy,
            PopupScreenStrategy::Terminal,
        );
    }

    #[test]
    fn tray_icon_style_defaults_to_colored_when_missing() {
        // 老 settings.json 没有 tray_icon_style 字段 → 默认彩色(原行为)
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"locale":"en"}"#).unwrap();
        let store = SettingsStore::load(path);
        assert_eq!(store.settings.tray_icon_style, TrayIconStyle::Colored);
    }

    #[test]
    fn popup_position_defaults_to_top_right_when_missing() {
        // 老 settings.json 没有 popup_position 字段 → 默认右上(原行为)
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"locale":"en"}"#).unwrap();
        let store = SettingsStore::load(path);
        assert_eq!(store.settings.popup_position, PopupPosition::TopRight);
    }

    #[test]
    fn popup_position_round_trips_kebab_case() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"locale":"en","popup_position":"bottom-right"}"#).unwrap();
        let store = SettingsStore::load(path);
        assert_eq!(store.settings.popup_position, PopupPosition::BottomRight);
    }

    #[test]
    fn popup_position_as_kebab_str() {
        assert_eq!(PopupPosition::TopLeft.as_kebab_str(), "top-left");
        assert_eq!(PopupPosition::TopRight.as_kebab_str(), "top-right");
        assert_eq!(PopupPosition::BottomLeft.as_kebab_str(), "bottom-left");
        assert_eq!(PopupPosition::BottomRight.as_kebab_str(), "bottom-right");
    }

    #[test]
    fn load_migrates_legacy_alert_sound_to_all_three_kinds() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"locale":"en","alert_sound":"system:Ping"}"#).unwrap();

        let store = SettingsStore::load(path);
        assert_eq!(store.settings.sound_pending, "system:Ping");
        assert_eq!(store.settings.sound_idle, "system:Ping");
        assert_eq!(store.settings.sound_failed, "system:Ping");
    }

    #[test]
    fn sound_for_status_maps_reminder_kinds() {
        use crate::sessions::SessionStatus;
        let settings = Settings::default();
        assert_eq!(
            settings.sound_for_status(SessionStatus::Pending),
            "builtin:waiting"
        );
        assert_eq!(
            settings.sound_for_status(SessionStatus::Idle),
            "builtin:completed"
        );
        assert_eq!(
            settings.sound_for_status(SessionStatus::LastFailed),
            "builtin:failed"
        );
    }

    #[test]
    fn load_fills_missing_fields_from_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"locale":"en"}"#).unwrap();

        let store = SettingsStore::load(path);
        assert_eq!(store.settings.locale, "en");
        assert_eq!(store.settings.sound_pending, "builtin:waiting");
        assert_eq!(store.settings.sound_idle, "builtin:completed");
        assert_eq!(store.settings.sound_failed, "builtin:failed");
        // 老配置文件（没有 notifications_enabled 字段）保持默认 true，避免静默变静音
        assert!(store.settings.notifications_enabled);
    }

    #[test]
    fn load_tolerates_unknown_legacy_fields() {
        // 老配置文件可能含已删除的 panel_shortcut / dev_mode / auto_start 字段;
        // serde 默认忽略未知字段,旧用户升级零成本。
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"locale":"en","panel_shortcut":"CmdOrCtrl+Shift+P","dev_mode":true,"auto_start":true}"#,
        )
        .unwrap();

        let store = SettingsStore::load(path);
        assert_eq!(store.settings.locale, "en");
    }

    #[test]
    fn load_returns_defaults_when_file_is_damaged() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{not valid json").unwrap();

        let store = SettingsStore::load(path);
        assert_eq!(store.settings.sound_pending, "builtin:waiting");
        assert_eq!(store.settings.sound_idle, "builtin:completed");
        assert_eq!(store.settings.sound_failed, "builtin:failed");
        // 损坏配置回落到 Settings::default();locale 随系统语言(default_locale),
        // 不能写死 "zh" —— 否则英文系统的 CI runner 会失败(见首启随系统语言功能)。
        assert_eq!(store.settings.locale, default_locale());
    }

    #[test]
    fn legacy_alert_sound_migrates_and_persists() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"alert_sound":"system:Ping"}"#).unwrap();

        let store = SettingsStore::load(path.clone());
        assert_eq!(store.settings.sound_pending, "system:Ping");
        assert_eq!(store.settings.sound_idle, "system:Ping");
        assert_eq!(store.settings.sound_failed, "system:Ping");
        // 迁移应落盘:磁盘收敛到新 schema(alert_sound 消失)
        let raw = std::fs::read_to_string(&path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(json["sound_pending"], "system:Ping");
        assert!(json.get("alert_sound").is_none());
    }

    #[test]
    fn load_does_not_rewrite_file_without_migration() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let original = r#"{"sound_pending":"builtin:waiting","sound_idle":"builtin:completed","sound_failed":"builtin:failed","locale":"en"}"#;
        std::fs::write(&path, original).unwrap();

        let _store = SettingsStore::load(path.clone());
        // 无需迁移时不落盘,文件原样保留
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }
}
