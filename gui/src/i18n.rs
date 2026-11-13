//! Small runtime string table. It deliberately covers the shell, Home and
//! first-run setup first; the remaining pages stay English until their
//! strings are translated, rather than showing machine-translated prose.

use eframe::egui;
use std::cell::Cell;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Language {
    English,
    Malay,
}

impl Language {
    pub fn from_str(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "ms" | "malay" | "bahasa" => Self::Malay,
            _ => Self::English,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Malay => "ms",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Malay => "Bahasa Melayu",
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

thread_local! {
    /// Tests render several languages in one process; this override keeps
    /// them from changing each other's assertions.
    static THREAD_LANGUAGE: Cell<Option<u8>> = const { Cell::new(None) };
}

#[cfg(test)]
pub fn set_thread(language: Option<Language>) {
    THREAD_LANGUAGE.with(|cell| cell.set(language.map(|l| l as u8)));
}

pub fn set(language: Language) {
    CURRENT.store(
        match language {
            Language::English => 0,
            Language::Malay => 1,
        },
        Ordering::Relaxed,
    );
}

pub fn current() -> Language {
    let code = THREAD_LANGUAGE
        .with(|cell| cell.get())
        .unwrap_or_else(|| CURRENT.load(Ordering::Relaxed));
    if code == 1 {
        Language::Malay
    } else {
        Language::English
    }
}

/// Pick a string for the active language. Keys are written as the English
/// text so an untranslated call site is obvious in review.
pub fn tr(english: &'static str, malay: &'static str) -> &'static str {
    match current() {
        Language::English => english,
        Language::Malay => malay,
    }
}

/// Translate a display string by its English text. Unknown strings fall back
/// to English, so a missing entry is visible, not broken.
pub fn t(text: &str) -> String {
    if current() == Language::English {
        return text.to_string();
    }
    let key = text.trim();
    let translated = match key {
        "Home" => "Utama",
        "My files" => "Fail saya",
        "Files" => "Fail",
        "Photos" => "Foto",
        "Activity" => "Aktiviti",
        "Settings" => "Tetapan",
        "Trash" => "Tong sampah",
        "Recovery" => "Pemulihan",
        "About" => "Perihal",
        "Sign in" => "Log masuk",
        "Sign out" => "Log keluar",
        "Sign in again" => "Log masuk semula",
        "Cancel" => "Batal",
        "Close" => "Tutup",
        "Save" => "Simpan",
        "Retry" => "Cuba lagi",
        "Back" => "Kembali",
        "Finish" => "Selesai",
        "Open" => "Buka",
        "Download" => "Muat turun",
        "Upload" => "Muat naik",
        "Rename" => "Namakan semula",
        "Delete" => "Padam",
        "Refresh" => "Muat semula",
        "Reload" => "Muat semula",
        "Clear" => "Kosongkan",
        "Search files..." => "Cari fail...",
        "Search file names..." => "Cari nama fail...",
        "Search log lines..." => "Cari baris log...",
        "New folder" => "Folder baharu",
        "New folder..." => "Folder baharu...",
        "Folder name:" => "Nama folder:",
        "New name:" => "Nama baharu:",
        "Name" => "Nama",
        "Size" => "Saiz",
        "Modified" => "Diubah",
        "Type" => "Jenis",
        "Folder" => "Folder",
        "File" => "Fail",
        "Open folder" => "Buka folder",
        "Open local folder" => "Buka folder tempatan",
        "Empty trash" => "Kosongkan tong sampah",
        "Restore" => "Pulihkan",
        "Delete permanently" => "Padam secara kekal",
        "Duplicate" => "Duplikasi",
        "Details" => "Butiran",
        "More" => "Lagi",
        "Move to..." => "Alih ke...",
        "Move to trash" => "Alih ke tong sampah",
        "Upload files..." => "Muat naik fail...",
        "Upload folder..." => "Muat naik folder...",
        "Select all" => "Pilih semua",
        "Clear selection" => "Kosongkan pilihan",
        "No files match" => "Tiada fail sepadan",
        "Clear search" => "Kosongkan carian",
        "This folder is empty" => "Folder ini kosong",
        "Drop files here or use Upload." => "Lepaskan fail di sini atau guna Muat naik.",
        "Sign in to load your files." => "Log masuk untuk memuatkan fail anda.",
        "Sign in to load your trash." => "Log masuk untuk memuatkan tong sampah anda.",
        "Sign in to see your files" => "Log masuk untuk melihat fail anda",
        "Sign in to see your trash" => "Log masuk untuk melihat tong sampah anda",
        "Your files will appear here after you sign in." => {
            "Fail anda akan muncul di sini selepas log masuk."
        }
        "Deleted files stay here until you sign in." => {
            "Fail yang dipadam kekal di sini sehingga anda log masuk."
        }
        "Your trash is empty" => "Tong sampah anda kosong",
        "Trash is empty." => "Tong sampah kosong.",
        "Cached names stay visible; the remote listing needs a session." => {
            "Nama cache kekal kelihatan; senarai jauh memerlukan sesi."
        }
        "Check your connection and try again." => "Semak sambungan anda dan cuba lagi.",
        "Couldn't open this folder." => "Tidak dapat membuka folder ini.",
        "Couldn't open this folder" => "Tidak dapat membuka folder ini",
        "Couldn't open" => "Tidak dapat membuka",
        "That didn't work." => "Itu tidak berjaya.",
        "Saved." => "Disimpan.",
        "Failed." => "Gagal.",
        "Loading..." => "Memuatkan...",
        "Loading folders..." => "Memuatkan folder...",
        "Loading the photo timeline..." => "Memuatkan garis masa foto...",
        "Checking..." => "Menyemak...",
        "Checking" => "Menyemak",
        "Starting" => "Memulakan",
        "Working" => "Sedang berjalan",
        "working" => "sedang berjalan",
        "Synced" => "Disegerakkan",
        "Not synced yet" => "Belum disegerakkan",
        "Paused" => "Dijeda",
        "Session expired" => "Sesi tamat",
        "Signing in" => "Sedang log masuk",
        "Engine stopped" => "Enjin berhenti",
        "Sync error" => "Ralat penyegerakan",
        "Sync now" => "Segerak sekarang",
        "Start syncing" => "Mula menyegerak",
        "Resume" => "Sambung",
        "Pause" => "Jeda",
        "Recent activity" => "Aktiviti terkini",
        "View all activity" => "Lihat semua aktiviti",
        "No activity yet — changes appear here as they sync." => {
            "Belum ada aktiviti — perubahan muncul di sini semasa disegerakkan."
        }
        "Technical log" => "Log teknikal",
        "Copy log" => "Salin log",
        "Log copied to clipboard." => "Log disalin ke papan keratan.",
        "Everything" => "Semua",
        "Uploaded" => "Dimuat naik",
        "Downloaded" => "Dimuat turun",
        "Files moved" => "Fail dialihkan",
        "Other" => "Lain-lain",
        "Filter" => "Tapis",
        "Filter activity by type" => "Tapis aktiviti mengikut jenis",
        "Dashboard" => "Papan pemuka",
        "Appearance" => "Penampilan",
        "Language / Bahasa" => "Bahasa / Language",
        "Theme" => "Tema",
        "Match my system" => "Ikut sistem saya",
        "Light" => "Cerah",
        "Dark" => "Gelap",
        "Interface size" => "Saiz antara muka",
        "Compact" => "Padat",
        "Small" => "Kecil",
        "Default" => "Lalai",
        "Large" => "Besar",
        "Extra large" => "Lebih besar",
        "Reset" => "Set semula",
        "General" => "Umum",
        "Sync" => "Segerak",
        "Notifications" => "Pemberitahuan",
        "Advanced" => "Lanjutan",
        "App & updates" => "Aplikasi & kemas kini",
        "Save all changes" => "Simpan semua perubahan",
        "Discard all changes" => "Buang semua perubahan",
        "Where your files live" => "Di mana fail anda disimpan",
        "What gets synced" => "Apa yang disegerakkan",
        "How often" => "Berapa kerap",
        "Performance" => "Prestasi",
        "Running in the background" => "Berjalan di latar belakang",
        "Installing on Linux" => "Memasang di Linux",
        "Updates" => "Kemas kini",
        "Check now" => "Semak sekarang",
        "Download and restart" => "Muat turun dan mulakan semula",
        "Open download page" => "Buka halaman muat turun",
        "Technical details" => "Butiran teknikal",
        "Copy diagnostics" => "Salin diagnostik",
        "Diagnostics copied." => "Diagnostik disalin.",
        "Credits" => "Kredit",
        "Licence (MIT)" => "Lesen (MIT)",
        "Third-party notices" => "Notis pihak ketiga",
        "Legal and trademarks" => "Undang-undang dan tanda dagangan",
        "Keyboard shortcuts" => "Pintasan papan kekunci",
        "What it does" => "Apa yang dilakukannya",
        "Good to know" => "Baik untuk diketahui",
        "Update available" => "Kemas kini tersedia",
        "Up to date" => "Terkini",
        "Not checked yet" => "Belum disemak",
        "No check has run yet." => "Belum ada semakan dijalankan.",
        "Update failed" => "Kemas kini gagal",
        "Check for new versions automatically" => "Semak versi baharu secara automatik",
        "Restart syncing now" => "Mulakan semula penyegerakan sekarang",
        "Open settings file" => "Buka fail tetapan",
        "Copy the log command" => "Salin arahan log",
        "Fine-tuning" => "Penalaan halus",
        "If something goes wrong" => "Jika sesuatu tidak kena",
        "Retries per folder" => "Cubaan semula setiap folder",
        "Scan workers" => "Pekerja imbasan",
        "Transfer concurrency" => "Keselarian pemindahan",
        "Run full check now" => "Jalankan semakan penuh sekarang",
        "Clear snapshot cache" => "Kosongkan cache petikan",
        "Send a test notification" => "Hantar pemberitahuan ujian",
        "This is what notifications look like." => "Beginilah rupa pemberitahuan.",
        "Keep this window above other windows" => "Kekalkan tetingkap ini di atas",
        "Keep running when I close the window" => {
            "Terus berjalan apabila saya tutup tetingkap"
        }
        "Start quietly in the tray" => "Mulakan secara senyap dalam dulang",
        "Start syncing when I log in" => "Mula menyegerak apabila saya log masuk",
        "Open Proton Drive when I log in" => "Buka Proton Drive apabila saya log masuk",
        "Add to my app menu" => "Tambah ke menu aplikasi saya",
        "Remove from app menu" => "Buang dari menu aplikasi",
        "Installed in your app menu." => "Dipasang dalam menu aplikasi anda.",
        "Not in your app menu yet." => "Belum berada dalam menu aplikasi anda.",
        "Extra environment variables" => "Pemboleh ubah persekitaran tambahan",
        "Photo timeline" => "Garis masa foto",
        "Photo folder" => "Folder foto",
        "Sync photos" => "Segerakkan foto",
        "Download all" => "Muat turun semua",
        "Re-download" => "Muat turun semula",
        "Duplicate name" => "Nama berganda",
        "No capture date" => "Tiada tarikh tangkapan",
        "No preview for this format" => "Tiada pratonton untuk format ini",
        "Preview failed" => "Pratonton gagal",
        "Size unknown" => "Saiz tidak diketahui",
        "Unknown" => "Tidak diketahui",
        "In the cloud" => "Di awan",
        "Proton Photos" => "Proton Photos",
        "Library size" => "Saiz pustaka",
        "All photos" => "Semua foto",
        "On this computer" => "Pada komputer ini",
        "Local copies" => "Salinan tempatan",
        "Photo sync off" => "Penyegerakan foto dimatikan",
        "Photo sync is off" => "Penyegerakan foto dimatikan",
        "Needs attention" => "Perlu perhatian",
        "Photo sync needs attention" => "Penyegerakan foto perlu perhatian",
        "Some photos need attention" => "Sesetengah foto perlu perhatian",
        "Syncing photos" => "Menyegerakkan foto",
        "Waiting for files" => "Menunggu fail",
        "Syncing your photos" => "Menyegerakkan foto anda",
        "Waiting for the file sync" => "Menunggu penyegerakan fail",
        "Photo sync is paused" => "Penyegerakan foto dijeda",
        "Resume on Home to continue." => "Sambung di Utama untuk meneruskan.",
        "Photo sync is stopped" => "Penyegerakan foto berhenti",
        "Your photo library" => "Pustaka foto anda",
        "No photos in your timeline yet." => "Belum ada foto dalam garis masa anda.",
        "Photos you upload from the Proton apps appear here." => {
            "Foto yang anda muat naik dari aplikasi Proton akan muncul di sini."
        }
        "Conflict copies" => "Salinan konflik",
        "No conflict copies" => "Tiada salinan konflik",
        "State snapshots" => "Petikan keadaan",
        "Create snapshot" => "Cipta petikan",
        "Open snapshot folder" => "Buka folder petikan",
        "Run recovery check" => "Jalankan semakan pemulihan",
        "Local trash" => "Tong sampah tempatan",
        "Open trash folder" => "Buka folder tong sampah",
        "Restore this snapshot" => "Pulihkan petikan ini",
        "Delete snapshot" => "Padam petikan",
        "Use conflict copy" => "Guna salinan konflik",
        "Keep current" => "Kekalkan semasa",
        "Keep both" => "Kekalkan kedua-dua",
        "Dismiss" => "Tutup",
        "Try again" => "Cuba lagi",
        "A native Linux home for your Proton Drive: two-way folder sync, a cloud file manager and a photo timeline, built on Proton's official command-line tool. Encryption happens on your device." => "Rumah Linux asli untuk Proton Drive anda: penyegerakan folder dua hala, pengurus fail awan dan garis masa foto, dibina atas alat baris perintah rasmi Proton. Penyulitan berlaku pada peranti anda.",
        "App version" => "Versi aplikasi",
        "A snapshot preserves the last-synced history, not your files. One is taken automatically before a root change, an approved mass delete, or an update." => "Petikan menyimpan sejarah penyegerakan terakhir, bukan fail anda. Petikan diambil secara automatik sebelum perubahan akar, pemadaman besar yang diluluskan, atau kemas kini.",
        "Built by the community on top of Proton's official command-line tool." => "Dibina oleh komuniti di atas alat baris perintah rasmi Proton.",
        "Choose a sync folder before reviewing conflicts." => "Pilih folder segerak sebelum menyemak konflik.",
        "Choose folder..." => "Pilih folder...",
        "Cloud file manager" => "Pengurus fail awan",
        "Community edition · an unofficial desktop client" => "Edisi komuniti · klien desktop tidak rasmi",
        "Conflicts could not be read" => "Konflik tidak dapat dibaca",
        "Conflicts keep both versions, so nothing is lost." => "Konflik menyimpan kedua-dua versi, jadi tiada yang hilang.",
        "Deletes go to trash by default; permanent delete is an explicit action." => "Pemadaman masuk tong sampah secara lalai; padam kekal ialah tindakan eksplisit.",
        "Files deleted locally are kept here until you restore or permanently delete them." => "Fail yang dipadam secara tempatan disimpan di sini sehingga anda memulihkan atau memadamnya secara kekal.",
        "GitHub repository" => "Repositori GitHub",
        "GUI build" => "Binaan GUI",
        "Keeps a local folder equal to Proton Drive in both directions." => "Memastikan folder tempatan sama dengan Proton Drive dalam kedua-dua arah.",
        "Last checked just now" => "Disemak sebentar tadi",
        "Loading local trash…" => "Memuatkan tong sampah tempatan…",
        "Loading snapshots…" => "Memuatkan petikan…",
        "No system tray is available on this desktop. Closing quits, and the quiet-start option is ignored." => "Tiada dulang sistem pada desktop ini. Menutup akan keluar, dan pilihan mula senyap diabaikan.",
        "Not scheduled" => "Tidak dijadualkan",
        "Not yet" => "Belum",
        "Official CLI" => "CLI rasmi",
        "Photos timeline" => "Garis masa foto",
        "Platform" => "Platform",
        "Proton CLI" => "Proton CLI",
        "Proton Docs and Sheets stay online only." => "Proton Docs dan Sheets kekal dalam talian sahaja.",
        "Proton Drive CLI" => "Proton Drive CLI",
        "Proton Drive for Linux" => "Proton Drive untuk Linux",
        "Review conflict copies and restore files that were deleted locally." => "Semak salinan konflik dan pulihkan fail yang dipadam secara tempatan.",
        "Review the timeline and keep local copies in step." => "Semak garis masa dan pastikan salinan tempatan seiring.",
        "Sign-in persists until you sign out." => "Log masuk kekal sehingga anda log keluar.",
        "The local edit is at the canonical name. The conflict copy is the cloud revision kept beside it." => "Suntingan tempatan berada pada nama kanonik. Salinan konflik ialah revisi awan yang disimpan di sebelahnya.",
        "Two-way folder sync" => "Penyegerakan folder dua hala",
        "Updates are signed and verified before they run." => "Kemas kini ditandatangani dan disahkan sebelum dijalankan.",
        "Update settings" => "Tetapan kemas kini",
        "Upload, download, rename, move and search from one list." => "Muat naik, muat turun, namakan semula, alih dan cari dari satu senarai.",
        "Your password is handled by Proton's official CLI, not this app." => "Kata laluan anda dikendalikan oleh CLI rasmi Proton, bukan aplikasi ini.",
        "NAME" => "NAMA",
        "SIZE" => "SAIZ",
        "MODIFIED" => "DIUBAH",
        "Loading local trash..." => "Memuatkan tong sampah tempatan...",
        "Loading snapshots..." => "Memuatkan petikan...",
        "Check folder" => "Semak folder",
        "Updated just now" => "Dikemas kini sebentar tadi",
        "Anywhere" => "Di mana-mana",
        "Choose this folder" => "Pilih folder ini",
        "Clear cache" => "Kosongkan cache",
        "Close a dialog" => "Tutup dialog",
        "In My Files" => "Dalam Fail Saya",
        "Move the selection to trash" => "Alih pilihan ke tong sampah",
        "My Files (/my-files)" => "Fail Saya (/my-files)",
        "OK" => "OK",
        "Open the focused folder or file" => "Buka folder atau fail yang difokus",
        "Open this list" => "Buka senarai ini",
        "Pause for 15 minutes" => "Jeda 15 minit",
        "Pause for 1 hour" => "Jeda 1 jam",
        "Pause until I resume" => "Jeda sehingga saya sambung",
        "Quit" => "Keluar",
        "Rename the selected item" => "Namakan semula item yang dipilih",
        "Search files" => "Cari fail",
        "Select all visible files" => "Pilih semua fail yang kelihatan",
        "Select or deselect the focused file" => "Pilih atau nyahpilih fail yang difokus",
        "Space" => "Space",
        "↑  Go up" => "↑  Naik",
        "Linux · Community edition" => "Linux · Edisi komuniti",
        "The sync folder or remote root changed. Saving archives the current sync history and clears cached cloud rows before the next sync." => "Folder segerak atau akar jauh berubah. Menyimpan akan mengarkibkan sejarah penyegerakan semasa dan mengosongkan baris awan cache sebelum penyegerakan seterusnya.",
        "Filter log lines..." => "Tapis baris log...",
        "For example:  *.tmp, .cache, *.iso" => "Contoh:  *.tmp, .cache, *.iso",
        "One KEY=VALUE per line, e.g.\nHTTPS_PROXY=http://proxy:3128" => "Satu KEY=VALUE setiap baris, cth.\nHTTPS_PROXY=http://proxy:3128",
        "Open Proton Drive" => "Buka Proton Drive",
        "Pause sync" => "Jeda penyegerakan",
        "Resume sync" => "Sambung penyegerakan",
        "Nothing yet" => "Belum ada",
        "Recent files" => "Fail terkini",
        "All done" => "Semua selesai",
        "Two versions were kept" => "Dua versi disimpan",
        "Sync problem" => "Masalah penyegerakan",
        "Checking your files..." => "Menyemak fail anda...",
        _ => {
            if let Some(out) = dynamic(key) {
                return out;
            }
            if std::env::var_os("PROTON_I18N_DEBUG").is_some() {
                eprintln!("i18n missing: {key:?}");
            }
            return text.to_string();
        }
    };
    translated.to_string()
}

/// Counts and relative times are built at runtime; translate their shape.
fn dynamic(text: &str) -> Option<String> {
    fn count(text: &str, suffix: &str, one: &str, many: &str) -> Option<String> {
        let n = text.strip_suffix(suffix)?;
        let n: u64 = n.parse().ok()?;
        Some(if n == 1 {
            one.replace("{n}", &n.to_string())
        } else {
            many.replace("{n}", &n.to_string())
        })
    }
    if let Some(s) = count(text, " items", "{n} item", "{n} item") {
        return Some(s);
    }
    if let Some(s) = count(text, " snapshots", "{n} petikan", "{n} petikan") {
        return Some(s);
    }
    if let Some(rest) = text.strip_prefix("Delete ")
        && let Some(n) = rest.strip_suffix(" files")
        && n.parse::<u64>().is_ok()
    {
        return Some(format!("Padam {n} fail"));
    }
    if let Some((n, rest)) = text.split_once(" photos · ")
        && let Ok(n) = n.parse::<u64>()
    {
        return Some(format!("{n} foto · {rest}"));
    }
    for (prefix, replacement) in [
        ("Conflict - both kept · ", "Konflik - kedua-dua disimpan · "),
        ("Uploaded · ", "Dimuat naik · "),
        ("Downloaded · ", "Dimuat turun · "),
        ("Folder created · ", "Folder dicipta · "),
        ("Removed · ", "Dibuang · "),
        ("Last checked ", "Disemak "),
        ("Next check in ", "Semakan seterusnya dalam "),
        ("Updated ", "Dikemas kini "),
        ("Every ", "Setiap "),
    ] {
        if let Some(rest) = text.strip_prefix(prefix) {
            return Some(format!("{replacement}{rest}"));
        }
    }
    None
}

/// `RichText` constructor that translates its text first. Call sites keep
/// their `.size()`, `.weak()` and other style chains.
pub fn rt(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(t(&text.into()))
}

/// `Ui` helpers used by the UI code for plain string literals. Styled
/// `RichText` call sites translate their inner string directly with `t()` so
/// their style survives.
#[allow(dead_code)] // complete UI surface; not every helper is called yet
pub trait TrUi {
    fn tlabel(&mut self, text: impl Into<String>) -> egui::Response;
    fn tbutton(&mut self, text: impl Into<String>) -> egui::Response;
    fn tsmall_button(&mut self, text: impl Into<String>) -> egui::Response;
    fn theading(&mut self, text: impl Into<String>) -> egui::Response;
    fn tcheckbox(&mut self, checked: &mut bool, text: impl Into<String>) -> egui::Response;
    fn tselectable_label(&mut self, selected: bool, text: impl Into<String>) -> egui::Response;
    fn tselectable_value<Value: PartialEq>(
        &mut self,
        current: &mut Value,
        value: Value,
        text: impl Into<String>,
    ) -> egui::Response;
}

impl TrUi for egui::Ui {
    fn tlabel(&mut self, text: impl Into<String>) -> egui::Response {
        self.label(t(&text.into()))
    }
    fn tbutton(&mut self, text: impl Into<String>) -> egui::Response {
        self.button(t(&text.into()))
    }
    fn tsmall_button(&mut self, text: impl Into<String>) -> egui::Response {
        self.small_button(t(&text.into()))
    }
    fn theading(&mut self, text: impl Into<String>) -> egui::Response {
        self.heading(t(&text.into()))
    }
    fn tcheckbox(&mut self, checked: &mut bool, text: impl Into<String>) -> egui::Response {
        self.checkbox(checked, t(&text.into()))
    }
    fn tselectable_label(&mut self, selected: bool, text: impl Into<String>) -> egui::Response {
        self.selectable_label(selected, t(&text.into()))
    }
    fn tselectable_value<Value: PartialEq>(
        &mut self,
        current: &mut Value,
        value: Value,
        text: impl Into<String>,
    ) -> egui::Response {
        self.selectable_value(current, value, t(&text.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malay_catalog_covers_core_surfaces() {
        set_thread(Some(Language::Malay));
        for (en, ms) in [
            ("Home", "Utama"),
            ("My files", "Fail saya"),
            ("Photos", "Foto"),
            ("Activity", "Aktiviti"),
            ("Settings", "Tetapan"),
            ("Recovery", "Pemulihan"),
            ("About", "Perihal"),
            ("Check folder", "Semak folder"),
            ("Sign in", "Log masuk"),
            ("Sync now", "Segerak sekarang"),
            ("No files match", "Tiada fail sepadan"),
            ("Conflict copies", "Salinan konflik"),
            ("Local trash", "Tong sampah tempatan"),
        ] {
            assert_eq!(t(en), ms, "missing translation for {en}");
        }
        assert_eq!(t("report.pdf"), "report.pdf");
        assert_eq!(t("3.33.0"), "3.33.0");
        set_thread(None);
    }

    #[test]
    fn language_codes_round_trip() {
        assert_eq!(Language::from_str("ms"), Language::Malay);
        assert_eq!(Language::from_str("en"), Language::English);
        assert_eq!(Language::from_str(""), Language::English);
        assert_eq!(Language::Malay.code(), "ms");
    }
}

