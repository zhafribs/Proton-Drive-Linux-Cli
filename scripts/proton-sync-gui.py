#!/usr/bin/env python3
"""
proton-sync-gui.py - "Proton Drive" desktop app for Linux (unofficial), v2.

Part of Proton-Drive-Linux-Cli (MIT). A Qt6 front-end for the two-way folder
sync service (proton-sync.py) on top of the OFFICIAL Proton Drive CLI.

Design ideas are taken from how professional desktop cloud-drive clients
(Proton Drive on Windows, Google Drive on Windows) behave and which settings
they offer - the implementation is 100% original MIT-licensed code in this
repository; no code, assets or trademarks of those products are used.

Pages
-----
  Dashboard  live status, next-check countdown, account, file count/size,
             Sync now / Open folder / Pause (15 min, 1 hour, until resumed)
  Changes    parsed transfer history (uploaded / downloaded / conflicts)
  Activity   raw live service log with filter
  Settings   sync, selective sync (folder picker), exclusions, notifications,
             startup, appearance, advanced environment
  Details    service/health statistics, paths, quick actions
  About      versions, links, license + legal notes

The engine writes ~/.local/state/proton-sync/health.json every cycle; the
dashboard reads that (instant) instead of parsing the journal each tick.

Run:  python3 scripts/proton-sync-gui.py      (or the proton-drive-gui shim)
"""

import datetime
import functools
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import traceback
from pathlib import Path

from PyQt6.QtCore import (QLockFile, QPoint, QSettings, QUrl, Qt, QThread,
                          QTimer, pyqtSignal as Signal)
from PyQt6.QtGui import (QCloseEvent, QColor, QDesktopServices, QIcon,
                         QPainter, QPalette, QPen, QPixmap)
from PyQt6.QtSvg import QSvgRenderer
from PyQt6.QtWidgets import (QApplication, QCheckBox, QComboBox, QFileDialog,
                             QFrame, QHBoxLayout, QLabel, QLineEdit, QListWidget,
                             QMenu, QMessageBox, QPlainTextEdit, QPushButton,
                             QScrollArea, QSizePolicy, QSpinBox, QStackedWidget,
                             QStyle, QSystemTrayIcon, QTreeWidget,
                             QTreeWidgetItem, QVBoxLayout, QWidget)

GUI_VERSION = "2.0.0"

REPO_URL = "https://github.com/zhafribs/Proton-Drive-Linux-Cli"
CONF_FILE = Path.home() / ".config" / "proton-sync.conf"
UNIT_FILE = Path.home() / ".config" / "systemd" / "user" / "proton-sync.service"
STATE_DIR = Path(os.environ.get("XDG_STATE_HOME", str(Path.home() / ".local/state"))) / "proton-sync"
HEALTH_FILE = STATE_DIR / "health.json"
WAKE_FILE = STATE_DIR / "wake"
LOCK_FILE = STATE_DIR / "gui.lock"
JOURNAL_UNIT = "proton-sync"

CONF_DEFAULTS = {
    "PROTON_SYNC_LOCAL": str(Path.home() / "ProtonDrive"),
    "PROTON_SYNC_REMOTE": "/my-files",
    "PROTON_SYNC_INTERVAL": "300",
    "PROTON_SYNC_DELETES": "0",
    "PROTON_SYNC_INOTIFY": "1",
    "PROTON_SYNC_WORKERS": "8",
    "PROTON_SYNC_RETRIES": "3",
    "PROTON_SYNC_SUBFOLDERS": "",
    "PROTON_SYNC_EXCLUDE": "",
}

ACCENT = "#6D4AFF"
STATUS_COLORS = {"synced": "#2e9e5b", "syncing": ACCENT, "paused": "#d9822b",
                 "error": "#d04444", "unknown": "#8a90a0"}
STATUS_LABELS = {"synced": "Up to date", "syncing": "Syncing...", "paused": "Paused",
                 "error": "Needs attention", "unknown": "Checking..."}

# --------------------------------------------------------------------------- helpers


def resolve_cli() -> str:
    """The same binary the service uses (unit Environment=, else PATH)."""
    try:
        text = UNIT_FILE.read_text(encoding="utf-8")
        m = re.search(r"^Environment=PROTON_DRIVE_BIN=(.+)$", text, re.M)
        if m and os.access(m.group(1).strip(), os.X_OK):
            return m.group(1).strip()
    except OSError:
        pass
    for cand in (shutil.which("proton-drive"), str(Path.home() / ".local/bin/proton-drive")):
        if cand and os.access(cand, os.X_OK):
            return cand
    return ""


def read_conf() -> dict:
    values = dict(CONF_DEFAULTS)
    try:
        for line in CONF_FILE.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                k, v = line.split("=", 1)
                values[k.strip()] = v.strip()
    except OSError:
        pass
    return values


def read_conf_extras() -> list:
    """KEY=VALUE lines in the conf that are not managed by this app."""
    out = []
    try:
        for line in CONF_FILE.read_text(encoding="utf-8").splitlines():
            s = line.strip()
            if s and not s.startswith("#") and "=" in s \
                    and s.split("=", 1)[0].strip() not in CONF_DEFAULTS:
                out.append(s)
    except OSError:
        pass
    return out


def write_conf(values: dict, extras: list = None):
    CONF_FILE.parent.mkdir(parents=True, exist_ok=True)
    parts = ["# proton-sync settings - editable by hand or with the Proton Drive GUI\n",
             "# changes apply after the service restarts\n"]
    for k, v in CONF_DEFAULTS.items():
        parts.append(f"{k}={values.get(k, v)}\n")
    for e in (extras or []):
        parts.append(e.rstrip() + "\n")
    CONF_FILE.write_text("".join(parts), encoding="utf-8")


def systemctl(*args, timeout=15) -> bool:
    try:
        return subprocess.run(["systemctl", "--user", *args],
                              capture_output=True, timeout=timeout).returncode == 0
    except (subprocess.TimeoutExpired, FileNotFoundError):
        return False


def systemd_run(*args, timeout=15) -> bool:
    exe = shutil.which("systemd-run")
    if not exe:
        return False
    try:
        return subprocess.run([exe, "--user", *args],
                              capture_output=True, timeout=timeout).returncode == 0
    except (subprocess.TimeoutExpired, FileNotFoundError):
        return False


def service_active() -> bool:
    exe = shutil.which("systemctl")
    if not exe:
        return False
    try:
        return subprocess.run([exe, "--user", "is-active", "--quiet", JOURNAL_UNIT],
                              timeout=10).returncode == 0
    except (subprocess.TimeoutExpired, FileNotFoundError):
        return False


ISO_RE = re.compile(r"^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}[+-]\d{4})")


def journal_lines(n=400) -> list:
    exe = shutil.which("journalctl")
    if not exe:
        return []
    try:
        r = subprocess.run([exe, "--user", "-u", JOURNAL_UNIT, "--no-pager",
                            "-n", str(n), "-o", "short-iso"],
                           capture_output=True, text=True, timeout=10)
        return r.stdout.splitlines()
    except (subprocess.TimeoutExpired, FileNotFoundError):
        return []


def journal_ts(line: str):
    m = ISO_RE.match(line)
    if not m:
        return None
    try:
        return datetime.datetime.strptime(m.group(1), "%Y-%m-%dT%H:%M:%S%z")
    except ValueError:
        return None


def journal_body(line: str) -> str:
    return line.split("]: ", 1)[-1] if "]: " in line else line


def read_health() -> dict:
    try:
        return json.loads(HEALTH_FILE.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}


def rel_time(dt) -> str:
    if dt is None:
        return "never"
    now = datetime.datetime.now(dt.tzinfo)
    secs = max(0, int((now - dt).total_seconds()))
    if secs < 60:
        return "just now"
    if secs < 3600:
        return f"{secs // 60} min ago"
    if secs < 86400:
        return f"{secs // 3600} h ago"
    return f"{secs // 86400} d ago"


def find_icon() -> str:
    here = Path(__file__).resolve().parent.parent / "src" / "protondrive-icon.svg"
    if here.is_file():
        return str(here)
    for cand in (Path.home() / ".local/share/icons/hicolor/scalable/apps/proton-drive-gui.svg",
                 Path("/usr/share/icons/hicolor/scalable/apps/proton-drive-gui.svg")):
        if cand.is_file():
            return str(cand)
    return ""


_PIXMAP_CACHE = {}


def svg_pixmap(path: str, size: int) -> QPixmap:
    key = (path, size)
    if key not in _PIXMAP_CACHE:
        pm = QPixmap(size, size)
        pm.fill(Qt.GlobalColor.transparent)
        if path and QSvgRenderer(path).isValid():
            QSvgRenderer(path).render(QPainter(pm))
        else:
            p = QPainter(pm)
            p.fillRect(0, 0, size, size, QColor(ACCENT))
            p.end()
        _PIXMAP_CACHE[key] = pm
    return _PIXMAP_CACHE[key]


def tray_icon(base: QPixmap, status: str) -> QIcon:
    pm = QPixmap(72, 72)
    pm.fill(Qt.GlobalColor.transparent)
    p = QPainter(pm)
    p.drawPixmap(6, 6, base.scaled(54, 54, Qt.AspectRatioMode.KeepAspectRatio,
                                   Qt.TransformationMode.SmoothTransformation))
    p.setPen(QPen(QColor("#ffffff"), 4))
    p.setBrush(QColor(STATUS_COLORS.get(status, "#8a90a0")))
    p.drawEllipse(48, 48, 22, 22)
    p.end()
    return QIcon(pm)


# --------------------------------------------------------------------------- workers


class Worker(QThread):
    """Run a slow function off the GUI thread."""
    done = Signal(object)

    def __init__(self, fn):
        super().__init__()
        self.fn = fn

    def run(self):
        try:
            self.done.emit(self.fn())
        except Exception as e:  # noqa: BLE001 - report anything to the UI
            self.done.emit(e)


def fetch_identity() -> dict:
    """CLI version + account email (one slow CLI call)."""
    out = {"version": "?", "email": None, "logged_in": False}
    cli = resolve_cli()
    if not cli:
        out["error"] = "official CLI not found"
        return out
    try:
        r = subprocess.run([cli, "--version"], capture_output=True, text=True, timeout=60)
        text = (r.stdout or r.stderr).strip()
        out["version"] = text.splitlines()[0] if text else "?"
    except subprocess.TimeoutExpired:
        out["version"] = "?"
    try:
        r = subprocess.run([cli, "fs", "list", "/my-files", "--json"],
                           capture_output=True, text=True, timeout=120)
        if r.returncode == 0 and r.stdout.strip():
            for e in json.loads(r.stdout):
                email = (e.get("ownedBy") or {}).get("email")
                if email:
                    out["email"] = email
                    out["logged_in"] = True
                    break
    except Exception:  # noqa: BLE001
        pass
    return out


def fetch_remote_folders(remote_root: str, rel_path: str) -> list:
    """Folders one level below REL_PATH, for the selective-sync picker."""
    cli = resolve_cli()
    if not cli:
        return []
    rpath = "/".join([remote_root] + [c.replace("/", r"\/") for c in rel_path.split("/")]) \
        if rel_path else remote_root
    r = subprocess.run([cli, "fs", "list", rpath, "--json"],
                       capture_output=True, text=True, timeout=120)
    if r.returncode != 0 or not r.stdout.strip():
        return []
    out = []
    for e in json.loads(r.stdout):
        if e.get("type") != "folder":
            continue
        name = (e.get("name") or {}).get("value")
        if not (e.get("name") or {}).get("ok", True) or not name:
            continue
        if re.match(r"^\.Trash-\d+$", name):
            continue
        out.append({"name": name, "rel": f"{rel_path}/{name}" if rel_path else name})
    return sorted(out, key=lambda f: f["name"].lower())


# --------------------------------------------------------------------------- styles

LIGHT = dict(BG="#f2f3f7", CARD="#ffffff", FG="#1c2029", MUTED="#6b7280",
             BORDER="#e4e6ec", SIDE="#171b26", SIDE_FG="#c9cdd8", SIDE_MUTED="#7d8494",
             INPUT="#ffffff", HOVER="#eceef4")
DARK = dict(BG="#14161c", CARD="#1d2029", FG="#e7e9ee", MUTED="#9aa1ad",
            BORDER="#2a2e3a", SIDE="#101218", SIDE_FG="#c9cdd8", SIDE_MUTED="#767d8c",
            INPUT="#161a22", HOVER="#242935")

QSS = """
QWidget {{ font-size: 13px; color: {FG}; }}
QFrame#sidebar {{ background: {SIDE}; border: none; }}
QLabel#appName {{ font-size: 17px; font-weight: 700; color: #ffffff; }}
QLabel#appSub {{ font-size: 11px; color: {SIDE_MUTED}; }}
QListWidget#nav {{ background: transparent; border: none; outline: none; }}
QListWidget#nav::item {{ padding: 10px 14px; margin: 3px 10px; border-radius: 8px;
                         color: {SIDE_FG}; }}
QListWidget#nav::item:hover {{ background: {HOVER}; }}
QListWidget#nav::item:selected {{ background: {ACCENT}; color: #ffffff; font-weight: 600; }}
QLabel#sideStatus {{ color: {SIDE_FG}; font-size: 12px; }}
QFrame#statusDot {{ border-radius: 6px; background: #8a90a0; min-width: 12px; max-width: 12px;
                    min-height: 12px; max-height: 12px; }}
QLabel#pageTitle {{ font-size: 22px; font-weight: 700; }}
QLabel#pageSub {{ color: {MUTED}; font-size: 12px; }}
QFrame#card {{ background: {CARD}; border: 1px solid {BORDER}; border-radius: 12px; }}
QLabel#cardTitle {{ color: {MUTED}; font-size: 11px; font-weight: 600; }}
QLabel#cardValue {{ font-size: 20px; font-weight: 700; }}
QLabel#statusTitle {{ font-size: 20px; font-weight: 700; }}
QLabel#statusDetail, QLabel#hint {{ color: {MUTED}; }}
QLabel#badge {{ padding: 3px 10px; border-radius: 10px; font-size: 11px; font-weight: 600; }}
QLabel#legal {{ color: {MUTED}; font-size: 12px; }}
QLabel#sectionHead {{ font-size: 14px; font-weight: 700; }}
QPushButton {{ background: {CARD}; border: 1px solid {BORDER}; border-radius: 8px;
               padding: 7px 14px; font-weight: 600; }}
QPushButton:hover {{ background: {HOVER}; }}
QPushButton:disabled {{ color: {MUTED}; }}
QPushButton#primary {{ background: {ACCENT}; border: none; color: #ffffff; }}
QPushButton#primary:hover {{ background: #5b3ce0; }}
QPushButton#danger {{ color: #d04444; }}
QLineEdit, QSpinBox, QPlainTextEdit, QComboBox, QTreeWidget {{
    background: {INPUT}; border: 1px solid {BORDER}; border-radius: 8px;
    padding: 6px 10px; selection-background-color: {ACCENT}; }}
QComboBox::drop-down {{ border: none; width: 22px; }}
QPlainTextEdit {{ font-family: "JetBrains Mono", "Fira Code", monospace; font-size: 12px; }}
QLineEdit:focus, QSpinBox:focus, QPlainTextEdit:focus, QTreeWidget:focus {{
    border: 1px solid {ACCENT}; }}
QCheckBox {{ spacing: 8px; }}
QCheckBox::indicator {{ width: 17px; height: 17px; }}
QTreeWidget {{ border: 1px solid {BORDER}; border-radius: 8px; }}
QTreeWidget::item {{ padding: 4px 2px; }}
QHeaderView::section {{ background: transparent; border: none; border-bottom: 1px solid {BORDER};
                        padding: 6px 8px; font-weight: 600; color: {MUTED}; }}
QScrollArea {{ border: none; }}
"""


def is_system_dark(app: QApplication) -> bool:
    c = app.palette().color(QPalette.ColorRole.Window)
    return (0.299 * c.red() + 0.587 * c.green() + 0.114 * c.blue()) < 128


def build_qss(dark: bool) -> str:
    v = dict(DARK if dark else LIGHT)
    v["ACCENT"] = ACCENT
    return QSS.format(**v)


# --------------------------------------------------------------------------- widgets


def guard_slot(fn):
    """A crashing slot must never take the whole app down."""
    @functools.wraps(fn)
    def wrapper(self, *a, **k):
        try:
            return fn(self, *a, **k)
        except Exception:  # noqa: BLE001
            traceback.print_exc()
    return wrapper


def card(title: str) -> tuple:
    frame = QFrame()
    frame.setObjectName("card")
    lay = QVBoxLayout(frame)
    lay.setContentsMargins(18, 14, 18, 14)
    lay.setSpacing(6)
    if title:
        t = QLabel(title.upper())
        t.setObjectName("cardTitle")
        lay.addWidget(t)
    return frame, lay


def stat_card(title: str, value: QLabel) -> QFrame:
    frame, lay = card(title)
    value.setObjectName("cardValue")
    value.setWordWrap(True)
    lay.addWidget(value)
    frame.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Fixed)
    return frame


def page_header(title: str, sub: str) -> QFrame:
    box = QFrame()
    lay = QVBoxLayout(box)
    lay.setContentsMargins(0, 0, 0, 4)
    lay.setSpacing(2)
    t = QLabel(title)
    t.setObjectName("pageTitle")
    s = QLabel(sub)
    s.setObjectName("pageSub")
    lay.addWidget(t)
    lay.addWidget(s)
    return box


def scroll_page(inner: QWidget) -> QScrollArea:
    sc = QScrollArea()
    sc.setWidgetResizable(True)
    sc.setWidget(inner)
    sc.setFrameShape(QFrame.Shape.NoFrame)
    return sc


def path_label(text: str) -> QLabel:
    lab = QLabel(text)
    lab.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
    lab.setObjectName("hint")
    lab.setWordWrap(True)
    return lab


# --------------------------------------------------------------------------- window

CHANGE_RE = re.compile(r"\b(UPLOAD|DOWNLOAD|MKDIR|TRASH|CONFLICT)\s*(.*)$")


class MainWindow(QWidget):
    def __init__(self):
        super().__init__()
        self.setWindowTitle("Proton Drive")
        self.setMinimumSize(980, 680)
        self.resize(1060, 720)
        self._quitting = False
        self._tray_warned = False
        self._identity = {"version": "?", "email": None, "logged_in": False}
        self._log_lines = []
        self._changes = []
        self._workers = []
        self._prefs = QSettings("ProtonDriveCLI", "ProtonDriveGUI")
        self._notify_cool = {}
        self._last_seen_ts = datetime.datetime.now().astimezone()
        self._prev_health_status = None
        self._selection = []
        self._remote_root = "/my-files"

        self.setWindowIcon(QIcon(svg_pixmap(find_icon(), 64)))

        self._build_ui()
        self._build_tray()

        self._status_timer = QTimer(self)
        self._status_timer.timeout.connect(self.refresh_status)
        self._status_timer.start(3000)

        self._log_timer = QTimer(self)
        self._log_timer.timeout.connect(self.refresh_logs)
        self._log_timer.start(3000)

        self._counts_timer = QTimer(self)
        self._counts_timer.timeout.connect(self.refresh_counts)
        self._counts_timer.start(15000)

        self._tick_timer = QTimer(self)
        self._tick_timer.timeout.connect(self.update_countdown)
        self._tick_timer.start(1000)

        self.refresh_status()
        self.refresh_logs()
        self.refresh_counts()
        self.refresh_details()
        self.load_identity()
        self.load_settings()

    # ------------------------------------------------------------------ UI build

    def _build_ui(self):
        root = QHBoxLayout(self)
        root.setContentsMargins(0, 0, 0, 0)
        root.setSpacing(0)

        side = QFrame()
        side.setObjectName("sidebar")
        side.setFixedWidth(216)
        sl = QVBoxLayout(side)
        sl.setContentsMargins(14, 18, 14, 16)
        sl.setSpacing(4)

        head = QHBoxLayout()
        head.setSpacing(10)
        glyph = QLabel()
        glyph.setPixmap(svg_pixmap(find_icon(), 30))
        head.addWidget(glyph)
        titles = QVBoxLayout()
        titles.setSpacing(0)
        name = QLabel("Proton Drive")
        name.setObjectName("appName")
        sub = QLabel("unofficial  ·  official CLI")
        sub.setObjectName("appSub")
        titles.addWidget(name)
        titles.addWidget(sub)
        head.addLayout(titles)
        head.addStretch(1)
        sl.addLayout(head)
        sl.addSpacing(18)

        self.nav = QListWidget()
        self.nav.setObjectName("nav")
        self.nav.setFixedHeight(200)
        for label in ("Dashboard", "Changes", "Activity", "Settings", "Details", "About"):
            self.nav.addItem(label)
        self.nav.currentRowChanged.connect(self._switch_page)
        sl.addWidget(self.nav)
        sl.addStretch(1)

        foot = QHBoxLayout()
        foot.setSpacing(8)
        self.side_dot = QFrame()
        self.side_dot.setObjectName("statusDot")
        foot.addWidget(self.side_dot)
        self.side_status = QLabel("Checking...")
        self.side_status.setObjectName("sideStatus")
        foot.addWidget(self.side_status)
        foot.addStretch(1)
        sl.addLayout(foot)
        self.side_version = QLabel("")
        self.side_version.setObjectName("appSub")
        sl.addWidget(self.side_version)

        root.addWidget(side)

        self.stack = QStackedWidget()
        root.addWidget(self.stack, 1)
        self.stack.addWidget(self._dashboard_page())
        self.stack.addWidget(self._changes_page())
        self.stack.addWidget(self._activity_page())
        self.stack.addWidget(self._settings_page())
        self.stack.addWidget(self._details_page())
        self.stack.addWidget(self._about_page())
        self.nav.setCurrentRow(0)

    # ---- dashboard ----------------------------------------------------------

    def _dashboard_page(self) -> QWidget:
        page = QWidget()
        outer = QVBoxLayout(page)
        outer.setContentsMargins(26, 22, 26, 18)
        outer.setSpacing(14)
        outer.addWidget(page_header("Dashboard", "Your Proton Drive at a glance"))

        inner = QWidget()
        col = QVBoxLayout(inner)
        col.setContentsMargins(0, 0, 6, 0)
        col.setSpacing(14)

        banner = QFrame()
        banner.setObjectName("card")
        bl = QHBoxLayout(banner)
        bl.setContentsMargins(20, 16, 20, 16)
        textcol = QVBoxLayout()
        textcol.setSpacing(3)
        self.status_title = QLabel("Checking...")
        self.status_title.setObjectName("statusTitle")
        self.status_detail = QLabel("")
        self.status_detail.setObjectName("statusDetail")
        textcol.addWidget(self.status_title)
        textcol.addWidget(self.status_detail)
        bl.addLayout(textcol, 1)

        btns = QHBoxLayout()
        btns.setSpacing(8)
        self.btn_sync = QPushButton(" Sync now")
        self.btn_sync.setIcon(self.style().standardIcon(QStyle.StandardPixmap.SP_BrowserReload))
        self.btn_sync.setObjectName("primary")
        self.btn_sync.clicked.connect(self.sync_now)
        self.btn_folder = QPushButton(" Open folder")
        self.btn_folder.setIcon(self.style().standardIcon(QStyle.StandardPixmap.SP_DirOpenIcon))
        self.btn_folder.clicked.connect(self.open_folder)
        self.btn_pause = QPushButton(" Pause")
        self.btn_pause.setIcon(self.style().standardIcon(QStyle.StandardPixmap.SP_MediaPause))
        self.btn_pause.clicked.connect(self._pause_clicked)
        for b in (self.btn_sync, self.btn_folder, self.btn_pause):
            btns.addWidget(b)
        bl.addLayout(btns)
        col.addWidget(banner)

        cards = QHBoxLayout()
        cards.setSpacing(12)
        self.val_files = QLabel("-")
        self.val_size = QLabel("-")
        self.val_sync = QLabel("-")
        self.val_service = QLabel("-")
        cards.addWidget(stat_card("Files in folder", self.val_files))
        cards.addWidget(stat_card("Folder size", self.val_size))
        cards.addWidget(stat_card("Last sync", self.val_sync))
        cards.addWidget(stat_card("Service", self.val_service))
        col.addLayout(cards)

        acct, al = card("Account")
        arow = QHBoxLayout()
        self.val_email = QLabel("...")
        self.val_email.setWordWrap(True)
        self.badge_login = QLabel("checking")
        self.badge_login.setObjectName("badge")
        arow.addWidget(self.val_email, 1)
        arow.addWidget(self.badge_login)
        self.btn_login = QPushButton("Log in")
        self.btn_login.clicked.connect(self.login_terminal)
        self.btn_logout = QPushButton("Log out")
        self.btn_logout.setObjectName("danger")
        self.btn_logout.clicked.connect(self.logout)
        arow.addWidget(self.btn_login)
        arow.addWidget(self.btn_logout)
        al.addLayout(arow)
        col.addWidget(acct)

        act, cl = card("Recent activity")
        self.preview = QPlainTextEdit()
        self.preview.setReadOnly(True)
        self.preview.document().setMaximumBlockCount(8)
        self.preview.setMinimumHeight(120)
        cl.addWidget(self.preview)
        col.addWidget(act)

        col.addStretch(1)
        outer.addWidget(scroll_page(inner), 1)
        return page

    # ---- changes ------------------------------------------------------------

    def _changes_page(self) -> QWidget:
        page = QWidget()
        outer = QVBoxLayout(page)
        outer.setContentsMargins(26, 22, 26, 18)
        outer.setSpacing(14)
        outer.addWidget(page_header("Changes", "What the sync has moved recently"))

        bar = QHBoxLayout()
        bar.setSpacing(8)
        self.ch_filter = QLineEdit()
        self.ch_filter.setPlaceholderText("filter by path...")
        self.ch_filter.textChanged.connect(self.render_changes)
        bar.addWidget(self.ch_filter, 1)
        self.ch_kind = QComboBox()
        self.ch_kind.addItems(["All changes", "Uploaded", "Downloaded", "Other"])
        self.ch_kind.currentIndexChanged.connect(self.render_changes)
        bar.addWidget(self.ch_kind)
        self.ch_count = QLabel("")
        self.ch_count.setObjectName("hint")
        bar.addWidget(self.ch_count)
        btn_copy = QPushButton("Copy list")
        btn_copy.clicked.connect(self.copy_changes)
        bar.addWidget(btn_copy)
        outer.addLayout(bar)

        self.ch_view = QTreeWidget()
        self.ch_view.setColumnCount(3)
        self.ch_view.setHeaderLabels(["When", "Action", "Path"])
        self.ch_view.setRootIsDecorated(False)
        self.ch_view.setAlternatingRowColors(True)
        self.ch_view.setUniformRowHeights(True)
        self.ch_view.header().setStretchLastSection(True)
        self.ch_view.setColumnWidth(0, 130)
        self.ch_view.setColumnWidth(1, 110)
        outer.addWidget(self.ch_view, 1)
        return page

    # ---- activity -----------------------------------------------------------

    def _activity_page(self) -> QWidget:
        page = QWidget()
        outer = QVBoxLayout(page)
        outer.setContentsMargins(26, 22, 26, 18)
        outer.setSpacing(14)
        outer.addWidget(page_header("Activity", "Live raw log of the sync service"))

        bar = QHBoxLayout()
        bar.setSpacing(8)
        self.log_filter = QLineEdit()
        self.log_filter.setPlaceholderText("filter log lines...")
        self.log_filter.textChanged.connect(self.render_logs)
        bar.addWidget(self.log_filter, 1)
        self.chk_auto = QCheckBox("auto-refresh")
        self.chk_auto.setChecked(True)
        bar.addWidget(self.chk_auto)
        btn_copy = QPushButton("Copy all")
        btn_copy.clicked.connect(self.copy_logs)
        bar.addWidget(btn_copy)
        btn_refresh = QPushButton("Refresh")
        btn_refresh.clicked.connect(self.refresh_logs)
        bar.addWidget(btn_refresh)
        outer.addLayout(bar)

        self.log_view = QPlainTextEdit()
        self.log_view.setReadOnly(True)
        self.log_view.document().setMaximumBlockCount(600)
        outer.addWidget(self.log_view, 1)
        return page

    # ---- settings -----------------------------------------------------------

    def _settings_page(self) -> QWidget:
        page = QWidget()
        outer = QVBoxLayout(page)
        outer.setContentsMargins(26, 22, 26, 18)
        outer.setSpacing(14)
        outer.addWidget(page_header("Settings", "Stored in ~/.config/proton-sync.conf"))

        inner = QWidget()
        col = QVBoxLayout(inner)
        col.setContentsMargins(0, 0, 6, 0)
        col.setSpacing(14)

        # --- sync
        box, form = card("Sync")
        form.setSpacing(10)

        row = QHBoxLayout()
        row.setSpacing(8)
        row.addWidget(QLabel("Local folder"))
        self.ed_local = QLineEdit()
        row.addWidget(self.ed_local, 1)
        btn_browse = QPushButton("Browse...")
        btn_browse.clicked.connect(self._browse)
        row.addWidget(btn_browse)
        form.addLayout(row)

        row = QHBoxLayout()
        row.setSpacing(8)
        row.addWidget(QLabel("Remote folder"))
        self.ed_remote = QLineEdit()
        self.ed_remote.setPlaceholderText("/my-files")
        row.addWidget(self.ed_remote, 1)
        form.addLayout(row)

        row = QHBoxLayout()
        row.setSpacing(8)
        row.addWidget(QLabel("Check for remote changes every"))
        self.sp_interval = QSpinBox()
        self.sp_interval.setRange(30, 3600)
        self.sp_interval.setSuffix(" seconds")
        row.addWidget(self.sp_interval)
        row.addStretch(1)
        form.addLayout(row)

        row = QHBoxLayout()
        row.setSpacing(8)
        row.addWidget(QLabel("Parallel scan connections"))
        self.sp_workers = QSpinBox()
        self.sp_workers.setRange(1, 16)
        row.addWidget(self.sp_workers)
        row.addSpacing(20)
        row.addWidget(QLabel("Retries per folder"))
        self.sp_retries = QSpinBox()
        self.sp_retries.setRange(1, 10)
        row.addWidget(self.sp_retries)
        row.addStretch(1)
        form.addLayout(row)

        self.chk_inotify = QCheckBox("Upload local changes instantly (watch the folder)")
        form.addWidget(self.chk_inotify)
        self.chk_deletes = QCheckBox("Also sync deletions (files deleted on one side get"
                                     " trashed on the other side)")
        form.addWidget(self.chk_deletes)
        hint = QLabel("Off = a file deleted on one side stays safely on the other side.")
        hint.setObjectName("hint")
        form.addWidget(hint)
        col.addWidget(box)

        # --- selective sync
        box, form = card("Selective sync")
        form.setSpacing(10)
        self.chk_selective = QCheckBox("Only sync selected folders (instead of everything)")
        form.addWidget(self.chk_selective)
        hint = QLabel("Checked folders are mirrored. Unchecked folders stay only in the "
                      "cloud and are not downloaded. Deselecting never deletes local files.")
        hint.setObjectName("hint")
        hint.setWordWrap(True)
        form.addWidget(hint)
        self.sel_tree = QTreeWidget()
        self.sel_tree.setHeaderHidden(True)
        self.sel_tree.setMinimumHeight(180)
        self.sel_tree.itemExpanded.connect(self._sel_expand)
        form.addWidget(self.sel_tree)
        row = QHBoxLayout()
        btn_reload = QPushButton("Reload folder list")
        btn_reload.clicked.connect(self._sel_load_root)
        row.addWidget(btn_reload)
        self.sel_hint = QLabel("")
        self.sel_hint.setObjectName("hint")
        row.addWidget(self.sel_hint, 1)
        form.addLayout(row)
        col.addWidget(box)

        # --- exclusions
        box, form = card("Exclusions")
        form.setSpacing(10)
        self.ed_exclude = QLineEdit()
        self.ed_exclude.setPlaceholderText("*.tmp, .cache, node_modules, *.iso")
        form.addWidget(self.ed_exclude)
        hint = QLabel("Comma-separated patterns, matched against file names and paths. "
                      "Matching files are neither uploaded nor downloaded.")
        hint.setObjectName("hint")
        hint.setWordWrap(True)
        form.addWidget(hint)
        col.addWidget(box)

        # --- notifications + startup
        box, form = card("Notifications and startup")
        form.setSpacing(8)
        self.chk_note_errors = QCheckBox("Notify me about sync errors")
        self.chk_note_conflicts = QCheckBox("Notify me about conflicts (both sides changed)")
        self.chk_note_done = QCheckBox("Notify me when a sync round finished with changes")
        self.chk_autostart = QCheckBox("Start syncing automatically at login")
        self.chk_starttray = QCheckBox("Start this app minimized in the tray")
        for w in (self.chk_note_errors, self.chk_note_conflicts, self.chk_note_done,
                  self.chk_autostart, self.chk_starttray):
            form.addWidget(w)
        row = QHBoxLayout()
        btn_test = QPushButton("Test notification")
        btn_test.clicked.connect(
            lambda: self.tray and self.tray.showMessage(
                "Proton Drive", "Notifications work.",
                QSystemTrayIcon.MessageIcon.Information, 3000))
        row.addWidget(btn_test)
        self.start_hint = QLabel("")
        self.start_hint.setObjectName("hint")
        row.addWidget(self.start_hint, 1)
        form.addLayout(row)
        col.addWidget(box)

        # --- appearance
        box, form = card("Appearance")
        row = QHBoxLayout()
        row.setSpacing(8)
        row.addWidget(QLabel("Theme"))
        self.cmb_theme = QComboBox()
        self.cmb_theme.addItems(["System", "Light", "Dark"])
        self.cmb_theme.currentIndexChanged.connect(self._theme_changed)
        row.addWidget(self.cmb_theme)
        row.addStretch(1)
        form.addLayout(row)
        col.addWidget(box)

        # --- advanced
        box, form = card("Advanced: extra environment variables")
        form.setSpacing(8)
        self.ed_extra = QPlainTextEdit()
        self.ed_extra.setPlaceholderText("HTTPS_PROXY=http://proxy:3128\nFOO=bar")
        self.ed_extra.setMinimumHeight(70)
        form.addWidget(self.ed_extra)
        hint = QLabel("One KEY=VALUE per line, passed to the sync service "
                      "(e.g. a proxy). Lines you do not manage here are preserved.")
        hint.setObjectName("hint")
        hint.setWordWrap(True)
        form.addWidget(hint)
        col.addWidget(box)

        btns = QHBoxLayout()
        btns.setSpacing(8)
        self.btn_save = QPushButton("Save settings")
        self.btn_save.setObjectName("primary")
        self.btn_save.clicked.connect(self.save_settings)
        btn_discard = QPushButton("Discard changes")
        btn_discard.clicked.connect(self.load_settings)
        btns.addWidget(self.btn_save)
        btns.addWidget(btn_discard)
        self.save_hint = QLabel("")
        self.save_hint.setObjectName("hint")
        btns.addWidget(self.save_hint)
        btns.addStretch(1)
        col.addLayout(btns)
        col.addStretch(1)

        outer.addWidget(scroll_page(inner))
        return page

    # ---- details ------------------------------------------------------------

    def _details_page(self) -> QWidget:
        page = QWidget()
        outer = QVBoxLayout(page)
        outer.setContentsMargins(26, 22, 26, 18)
        outer.setSpacing(14)
        outer.addWidget(page_header("Details", "Service health, statistics and paths"))

        inner = QWidget()
        col = QVBoxLayout(inner)
        col.setContentsMargins(0, 0, 6, 0)
        col.setSpacing(14)

        box, form = card("Service")
        form.setSpacing(6)
        self.det_service = QLabel("-")
        self.det_cycle = QLabel("-")
        self.det_entries = QLabel("-")
        self.det_next = QLabel("-")
        for lab in (self.det_service, self.det_cycle, self.det_entries, self.det_next):
            lab.setWordWrap(True)
            form.addWidget(lab)
        row = QHBoxLayout()
        row.setSpacing(8)
        btn_restart = QPushButton("Restart service now")
        btn_restart.clicked.connect(self.restart_service)
        row.addWidget(btn_restart)
        btn_open_conf = QPushButton("Open config file")
        btn_open_conf.clicked.connect(
            lambda: QDesktopServices.openUrl(QUrl.fromLocalFile(str(CONF_FILE))))
        row.addWidget(btn_open_conf)
        btn_journal = QPushButton("Copy log command")
        btn_journal.clicked.connect(
            lambda: (QApplication.clipboard().setText("journalctl --user -u proton-sync -f"),
                     self.sel_hint.setText("copied")))
        row.addWidget(btn_journal)
        row.addStretch(1)
        form.addLayout(row)
        col.addWidget(box)

        box, form = card("Paths")
        form.setSpacing(4)
        for title, value in (
                ("Sync folder", self._local_path()),
                ("Settings file", str(CONF_FILE)),
                ("Health / state", str(STATE_DIR)),
                ("Service unit", str(UNIT_FILE)),
        ):
            form.addWidget(QLabel(title))
            form.addWidget(path_label(value))
        col.addWidget(box)

        box, form = card("Versions")
        form.setSpacing(4)
        self.det_versions = QLabel("-")
        self.det_versions.setWordWrap(True)
        form.addWidget(self.det_versions)
        col.addWidget(box)

        col.addStretch(1)
        outer.addWidget(scroll_page(inner))
        return page

    # ---- about --------------------------------------------------------------

    def _about_page(self) -> QWidget:
        page = QWidget()
        outer = QVBoxLayout(page)
        outer.setContentsMargins(26, 22, 26, 18)
        outer.setSpacing(14)
        outer.addWidget(page_header("About", "Proton Drive folder app for Linux"))

        inner = QWidget()
        col = QVBoxLayout(inner)
        col.setContentsMargins(0, 0, 6, 0)
        col.setSpacing(14)

        box, lay = card("This app")
        line = QHBoxLayout()
        line.setSpacing(14)
        big = QLabel()
        big.setPixmap(svg_pixmap(find_icon(), 56))
        line.addWidget(big)
        texts = QVBoxLayout()
        texts.setSpacing(2)
        t = QLabel("Proton Drive")
        t.setObjectName("cardValue")
        self.about_version = QLabel("GUI " + GUI_VERSION + "  ·  loading CLI version...")
        self.about_version.setObjectName("hint")
        texts.addWidget(t)
        texts.addWidget(self.about_version)
        line.addLayout(texts, 1)
        btn_repo = QPushButton("Open GitHub repository")
        btn_repo.clicked.connect(lambda: QDesktopServices.openUrl(QUrl(REPO_URL)))
        line.addWidget(btn_repo)
        btn_copy = QPushButton("Copy link")
        btn_copy.clicked.connect(lambda: QApplication.clipboard().setText(REPO_URL))
        line.addWidget(btn_copy)
        lay.addLayout(line)
        how = QLabel(
            "This window is a friendly front-end for the <b>official Proton Drive CLI</b> "
            "and the sync service that keeps <b>~/ProtonDrive</b> equal to your cloud. "
            "Its feature set is modeled on what professional desktop cloud-drive clients "
            "do - the code is original MIT-licensed work in this repository.")
        how.setObjectName("legal")
        how.setWordWrap(True)
        lay.addWidget(how)
        col.addWidget(box)

        legal, ll = card("License and legal")
        txt = QLabel(
            "<b>Unofficial.</b> Not affiliated with, endorsed, sponsored or supported by "
            "Proton AG, Google LLC or Microsoft Corporation. This app talks to the official "
            "Proton Drive CLI (its payload is MIT, Copyright (c) 2025-2026 Proton AG; source: "
            "github.com/ProtonDriveApps/sdk). This repository's own scripts, including this "
            "GUI, are MIT (see LICENSE in the repo). 'Proton' and 'Proton Drive' are "
            "trademarks of Proton AG, used here only to identify the product being packaged. "
            "References to other products are feature comparisons only - no code, assets or "
            "configuration of those products is used. No AUR, no third-party binaries."
            "<br><br>Not legal advice.")
        txt.setObjectName("legal")
        txt.setWordWrap(True)
        ll.addWidget(txt)
        col.addWidget(legal)
        col.addStretch(1)

        outer.addWidget(scroll_page(inner))
        return page

    # ------------------------------------------------------------------ tray

    def _build_tray(self):
        self.tray = None
        if not QSystemTrayIcon.isSystemTrayAvailable():
            return
        self.tray = QSystemTrayIcon(tray_icon(svg_pixmap(find_icon(), 64), "unknown"), self)
        self.tray.setToolTip("Proton Drive")
        menu = QMenu()
        act_show = menu.addAction("Show window")
        act_show.triggered.connect(self.show_normal)
        menu.addSeparator()
        self.act_sync = menu.addAction("Sync now")
        self.act_sync.triggered.connect(self.sync_now)
        act_folder = menu.addAction("Open folder")
        act_folder.triggered.connect(self.open_folder)
        menu.addSeparator()
        self.act_pause = menu.addAction("Pause sync")
        self.act_pause.triggered.connect(self._pause_clicked)
        menu.addSeparator()
        act_quit = menu.addAction("Quit")
        act_quit.triggered.connect(self.quit_app)
        self.tray.setContextMenu(menu)
        self.tray.activated.connect(self._tray_activated)
        self.tray.show()

    def _tray_activated(self, reason):
        if reason == QSystemTrayIcon.ActivationReason.Trigger:
            self.show_normal()

    def show_normal(self):
        self.showNormal()
        self.raise_()
        self.activateWindow()

    # ------------------------------------------------------------------ status

    def _switch_page(self, row: int):
        self.stack.setCurrentIndex(max(0, row))
        if row == 1:
            self.render_changes()

    @guard_slot
    def refresh_status(self):
        active = service_active()
        health = read_health()
        hstatus = health.get("status", "")
        if not active:
            status = "paused"
        elif hstatus in ("ok", "syncing", "error"):
            status = {"ok": "synced", "syncing": "syncing", "error": "error"}[hstatus]
        else:
            status = "unknown"
        color = STATUS_COLORS.get(status, STATUS_COLORS["unknown"])

        self.status_title.setText(STATUS_LABELS.get(status, status))
        if status == "error":
            detail = str(health.get("error") or "see Activity page for details")
        elif status == "paused":
            detail = "Sync is paused. Press Resume to continue."
        elif status == "syncing":
            started = str(health.get("started", ""))[11:19]
            detail = f"checking and syncing since {started}" if started else "working..."
        else:
            detail = self._next_poll_text()
        self.status_detail.setText(detail[:220])
        self.status_title.setStyleSheet(f"color: {color};")

        self.side_dot.setStyleSheet(
            f"border-radius: 6px; background: {color}; min-width:12px; max-width:12px;"
            "min-height:12px; max-height:12px;")
        self.side_status.setText(STATUS_LABELS.get(status, status))

        self.val_service.setText("running" if active else "stopped")
        try:
            ts = datetime.datetime.fromisoformat(str(health.get("ts")))
            self.val_sync.setText(rel_time(ts) if hstatus == "ok" else "-")
        except ValueError:
            self.val_sync.setText("-")

        self.btn_pause.setText(" Pause" if active else " Resume")
        self.btn_pause.setIcon(self.style().standardIcon(
            QStyle.StandardPixmap.SP_MediaPause if active else QStyle.StandardPixmap.SP_MediaPlay))
        self.btn_sync.setEnabled(active)
        self.act_sync.setEnabled(active)
        self.act_pause.setText("Pause sync" if active else "Resume sync")

        if self.tray:
            self.tray.setIcon(tray_icon(svg_pixmap(find_icon(), 64), status))
            self.tray.setToolTip(f"Proton Drive - {STATUS_LABELS.get(status, status)}")

        self._poll_notifications(health)
        self._prev_health_status = hstatus

    def _next_poll_text(self) -> str:
        health = read_health()
        try:
            nxt = datetime.datetime.fromisoformat(str(health.get("next_poll")))
            secs = max(0, int((nxt - datetime.datetime.now(nxt.tzinfo)).total_seconds()))
            return f"next check in ~{secs // 60}m {secs % 60:02d}s  ·  in sync"
        except ValueError:
            return "in sync"

    @guard_slot
    def update_countdown(self):
        if self.status_title.text() == STATUS_LABELS["synced"]:
            self.status_detail.setText(self._next_poll_text())
        if self.stack.currentWidget() is not None and self.nav.currentRow() == 4:
            self.det_next.setText("Next check: " + self._next_poll_text())

    @guard_slot
    def refresh_logs(self):
        self._log_lines = journal_lines(400)
        self.preview.setPlainText("\n".join(self._log_lines[-8:]) or "(no activity yet)")
        self._parse_changes()
        if self.chk_auto.isChecked():
            self.render_logs()
        if self.nav.currentRow() == 1:
            self.render_changes()

    def render_logs(self):
        needle = self.log_filter.text().strip().lower()
        lines = [ln for ln in self._log_lines if needle in ln.lower()] if needle else self._log_lines
        at_bottom = (self.log_view.verticalScrollBar().value()
                     >= self.log_view.verticalScrollBar().maximum() - 4)
        self.log_view.setPlainText("\n".join(lines[-600:]))
        if at_bottom or needle:
            sb = self.log_view.verticalScrollBar()
            sb.setValue(sb.maximum())

    def copy_logs(self):
        QApplication.clipboard().setText("\n".join(self._log_lines))

    def _parse_changes(self):
        changes = []
        for ln in self._log_lines[-400:]:
            body = journal_body(ln)
            m = CHANGE_RE.search(body)
            if not m:
                continue
            ts = journal_ts(ln)
            kind, rest = m.group(1), m.group(2).strip()
            if kind == "UPLOAD":
                label, direction = "Uploaded", "up"
            elif kind == "DOWNLOAD":
                label, direction = "Downloaded", "down"
                rest = re.sub(r"^dir\s+", "", rest)
            elif kind == "MKDIR":
                label, direction = "Folder created", "up"
            elif kind == "TRASH":
                label, direction = "Trashed", "other"
            else:
                label, direction = "Conflict", "warn"
            rest = re.sub(r"\s*\(.*\)$", "", rest)
            changes.append({"ts": ts, "label": label, "dir": direction, "path": rest})
        self._changes = changes[-300:]

    def render_changes(self):
        kind = self.ch_kind.currentIndex()
        needle = self.ch_filter.text().strip().lower()
        self.ch_view.clear()
        shown = 0
        for c in reversed(self._changes):
            if kind == 1 and c["dir"] != "up":
                continue
            if kind == 2 and c["dir"] != "down":
                continue
            if kind == 3 and c["dir"] not in ("other", "warn"):
                continue
            if needle and needle not in c["path"].lower():
                continue
            when = c["ts"].strftime("%d %b %H:%M:%S") if c["ts"] else "?"
            item = QTreeWidgetItem([when, c["label"], c["path"]])
            color = {"up": "#2e9e5b", "down": "#3b6fd4", "warn": "#d9822b",
                     "other": "#8a90a0"}.get(c["dir"], "#8a90a0")
            item.setForeground(1, QColor(color))
            self.ch_view.addTopLevelItem(item)
            shown += 1
            if shown >= 300:
                break
        self.ch_count.setText(f"{shown} entries")

    def copy_changes(self):
        text = "\n".join(f"{c['ts'].strftime('%Y-%m-%d %H:%M:%S') if c['ts'] else ''}"
                         f"\t{c['label']}\t{c['path']}" for c in reversed(self._changes))
        QApplication.clipboard().setText(text)
        self.ch_count.setText("copied")

    @guard_slot
    def refresh_counts(self):
        root = Path(self.ed_local.text().strip() or CONF_DEFAULTS["PROTON_SYNC_LOCAL"]).expanduser()
        files = 0
        total = 0
        try:
            for dirpath, _dirnames, filenames in os.walk(root):
                for f in filenames:
                    try:
                        total += os.path.getsize(os.path.join(dirpath, f))
                        files += 1
                    except OSError:
                        pass
        except OSError:
            pass
        self.val_files.setText(f"{files:,}")
        self.val_size.setText(f"{total / 1_048_576:,.0f} MiB")

    @guard_slot
    def refresh_details(self):
        active = service_active()
        health = read_health()
        self.det_service.setText(
            "Service: " + ("running" if active else "stopped")
            + f"   ·   state: {health.get('status', '?')}")
        bits = []
        if "cycle_seconds" in health:
            bits.append(f"last cycle took {health['cycle_seconds']} s")
        if "actions" in health:
            bits.append(f"{health['actions']} action(s), {health.get('failed', 0)} failed")
        if "uploaded" in health or "downloaded" in health:
            bits.append(f"last round: {health.get('uploaded', 0)} uploaded, "
                        f"{health.get('downloaded', 0)} downloaded, "
                        f"{health.get('conflicts', 0)} conflicts")
        if "holds" in health:
            bits.append(f"{health['holds']} unchanged")
        self.det_cycle.setText("   ·   ".join(bits) or "-")
        self.det_entries.setText(
            f"counted: {health.get('remote_entries', '?')} remote / "
            f"{health.get('local_entries', '?')} local / "
            f"{health.get('state_records', '?')} state records")
        self.det_next.setText("Next check: " + self._next_poll_text())
        self.det_versions.setText(
            f"GUI {GUI_VERSION}  ·  CLI {self._identity.get('version', '?')}  ·  "
            f"Python {sys.version.split()[0]}  ·  Qt {self._qt_version()}")

    @staticmethod
    def _qt_version() -> str:
        from PyQt6.QtCore import QT_VERSION_STR
        return QT_VERSION_STR

    # ------------------------------------------------------------------ actions

    def sync_now(self):
        if not service_active():
            QMessageBox.information(self, "Sync paused",
                                    "The sync service is paused. Press Resume first.")
            return
        STATE_DIR.mkdir(parents=True, exist_ok=True)
        WAKE_FILE.touch()
        self.status_detail.setText("manual sync requested...")

    def _local_path(self) -> str:
        return self.ed_local.text().strip() if hasattr(self, "ed_local") \
            else read_conf()["PROTON_SYNC_LOCAL"]

    def open_folder(self):
        root = Path(self._local_path() or CONF_DEFAULTS["PROTON_SYNC_LOCAL"]).expanduser()
        root.mkdir(parents=True, exist_ok=True)
        QDesktopServices.openUrl(QUrl.fromLocalFile(str(root)))

    def _browse(self):
        start = self.ed_local.text().strip() or str(Path.home())
        chosen = QFileDialog.getExistingDirectory(self, "Choose the folder to sync", start)
        if chosen:
            self.ed_local.setText(chosen)

    def _pause_clicked(self):
        if not service_active():
            self.resume_now()
            return
        menu = QMenu(self)
        menu.addAction("Pause for 15 minutes", lambda: self.pause_for(15))
        menu.addAction("Pause for 1 hour", lambda: self.pause_for(60))
        menu.addAction("Pause until I resume", lambda: self.pause_for(None))
        menu.exec(self.btn_pause.mapToGlobal(QPoint(0, self.btn_pause.height())))

    def pause_for(self, minutes):
        systemctl("stop", JOURNAL_UNIT)
        if minutes:
            systemctl("stop", "proton-sync-autoresume.timer", "proton-sync-autoresume.service")
            systemd_run(f"--on-active={minutes * 60}", "--unit=proton-sync-autoresume",
                         shutil.which("systemctl") or "systemctl", "--user", "start", JOURNAL_UNIT)
            self.status_detail.setText(f"paused for {minutes} minute(s)")
        else:
            systemctl("stop", "proton-sync-autoresume.timer", "proton-sync-autoresume.service")
            self.status_detail.setText("paused")
        QTimer.singleShot(600, self.refresh_status)

    def resume_now(self):
        systemctl("stop", "proton-sync-autoresume.timer", "proton-sync-autoresume.service")
        systemctl("start", JOURNAL_UNIT)
        QTimer.singleShot(600, self.refresh_status)

    def restart_service(self):
        systemctl("restart", JOURNAL_UNIT)
        QTimer.singleShot(800, self.refresh_status)
        self.det_service.setText("Service: restarting...")

    # ------------------------------------------------------------------ settings

    @guard_slot
    def load_settings(self):
        c = read_conf()
        self.ed_local.setText(c["PROTON_SYNC_LOCAL"])
        self.ed_remote.setText(c["PROTON_SYNC_REMOTE"])
        self._remote_root = c["PROTON_SYNC_REMOTE"] or "/my-files"
        self.sp_interval.setValue(int(c["PROTON_SYNC_INTERVAL"]))
        self.chk_inotify.setChecked(c["PROTON_SYNC_INOTIFY"] == "1")
        self.chk_deletes.setChecked(c["PROTON_SYNC_DELETES"] == "1")
        self.sp_workers.setValue(int(c["PROTON_SYNC_WORKERS"]))
        self.sp_retries.setValue(int(c["PROTON_SYNC_RETRIES"]))
        self.ed_exclude.setText(c["PROTON_SYNC_EXCLUDE"])
        self._selection = [s for s in c["PROTON_SYNC_SUBFOLDERS"].split(",") if s]
        self.chk_selective.setChecked(bool(self._selection))
        self.ed_extra.setPlainText("\n".join(read_conf_extras()))
        self.save_hint.setText("")

        # preferences (GUI-only, stored via QSettings)
        theme = self._prefs.value("theme", "system")
        self.cmb_theme.blockSignals(True)
        self.cmb_theme.setCurrentText({"system": "System", "light": "Light",
                                       "dark": "Dark"}.get(theme, "System"))
        self.cmb_theme.blockSignals(False)
        self.chk_starttray.setChecked(self._prefs.value("start_in_tray", False, type=bool))
        self.chk_note_errors.setChecked(self._prefs.value("notify_errors", True, type=bool))
        self.chk_note_conflicts.setChecked(self._prefs.value("notify_conflicts", True, type=bool))
        self.chk_note_done.setChecked(self._prefs.value("notify_complete", False, type=bool))

        exe = shutil.which("systemctl")
        enabled = False
        if exe:
            try:
                enabled = subprocess.run([exe, "--user", "is-enabled", "--quiet", JOURNAL_UNIT],
                                         timeout=10).returncode == 0
            except (subprocess.TimeoutExpired, FileNotFoundError):
                pass
        self.chk_autostart.blockSignals(True)
        self.chk_autostart.setChecked(enabled)
        self.chk_autostart.blockSignals(False)

        self._sel_load_root()

    def save_settings(self):
        local = self.ed_local.text().strip()
        if not local.startswith("/"):
            QMessageBox.warning(self, "Invalid folder", "The local folder must be an absolute path.")
            return
        selection = self._derive_selection() if self.chk_selective.isChecked() else []
        values = {
            "PROTON_SYNC_LOCAL": local,
            "PROTON_SYNC_REMOTE": self.ed_remote.text().strip() or "/my-files",
            "PROTON_SYNC_INTERVAL": str(self.sp_interval.value()),
            "PROTON_SYNC_DELETES": "1" if self.chk_deletes.isChecked() else "0",
            "PROTON_SYNC_INOTIFY": "1" if self.chk_inotify.isChecked() else "0",
            "PROTON_SYNC_WORKERS": str(self.sp_workers.value()),
            "PROTON_SYNC_RETRIES": str(self.sp_retries.value()),
            "PROTON_SYNC_SUBFOLDERS": ",".join(selection),
            "PROTON_SYNC_EXCLUDE": self.ed_exclude.text().strip(),
        }
        extras = [l.strip() for l in self.ed_extra.toPlainText().splitlines()
                  if l.strip() and not l.strip().startswith("#")
                  and "=" in l and l.split("=", 1)[0].strip() not in CONF_DEFAULTS]
        answer = QMessageBox.question(
            self, "Save settings",
            "Save and restart the sync service now?\n\n"
            "(An upload or download in progress will simply continue in the next cycle.)")
        if answer != QMessageBox.StandardButton.Yes:
            return
        write_conf(values, extras)
        outdated = True
        try:
            outdated = "EnvironmentFile" not in UNIT_FILE.read_text(encoding="utf-8")
        except OSError:
            pass
        restarted = systemctl("restart", JOURNAL_UNIT)
        if outdated:
            self.save_hint.setText("saved, but the service unit is old - run "
                                   "./scripts/install-sync.sh in the repo")
        elif restarted:
            self.save_hint.setText("saved - service restarted")
        else:
            self.save_hint.setText("saved, but the service could not be restarted")
        QTimer.singleShot(800, self.refresh_status)

    def _theme_changed(self, index: int):
        name = ["system", "light", "dark"][max(0, min(2, index))]
        self._prefs.setValue("theme", name)
        self.apply_theme(name)

    @staticmethod
    def apply_theme(name: str):
        app = QApplication.instance()
        if not app:
            return
        dark = is_system_dark(app) if name == "system" else (name == "dark")
        app.setStyleSheet(build_qss(dark))

    # --- selective sync tree

    def _sel_load_root(self):
        self.sel_tree.clear()
        c = read_conf()
        self._remote_root = c["PROTON_SYNC_REMOTE"] or "/my-files"
        self._selection = [s for s in c["PROTON_SYNC_SUBFOLDERS"].split(",") if s]
        root = QTreeWidgetItem([f"My files  ({self._remote_root})"])
        root.setData(0, Qt.ItemDataRole.UserRole, "")
        root.setFlags(root.flags() | Qt.ItemFlag.ItemIsUserCheckable
                      | Qt.ItemFlag.ItemIsEnabled | Qt.ItemFlag.ItemIsAutoTristate)
        root.setCheckState(0, Qt.CheckState.Unchecked if self._selection
                           else Qt.CheckState.Checked)
        self.sel_tree.addTopLevelItem(root)
        placeholder = QTreeWidgetItem(root, ["Loading..."])
        placeholder.setData(0, Qt.ItemDataRole.UserRole, None)
        self.sel_hint.setText("loading folder list...")

        w = Worker(lambda: fetch_remote_folders(self._remote_root, ""))
        w.done.connect(lambda r: self._sel_populate(root, r))
        self._workers.append(w)
        w.start()

    def _sel_populate(self, item, folders):
        if isinstance(folders, Exception):
            folders = []
        item.takeChildren()
        sel = set(self._selection)
        for f in folders:
            child = QTreeWidgetItem(item, [f["name"]])
            child.setData(0, Qt.ItemDataRole.UserRole, f["rel"])
            child.setData(0, Qt.ItemDataRole.UserRole + 1, True)
            child.setFlags(child.flags() | Qt.ItemFlag.ItemIsUserCheckable
                           | Qt.ItemFlag.ItemIsEnabled | Qt.ItemFlag.ItemIsAutoTristate)
            child.setCheckState(0, Qt.CheckState.Checked if f["rel"] in sel
                                else Qt.CheckState.Unchecked)
            QTreeWidgetItem(child, ["..."]).setData(0, Qt.ItemDataRole.UserRole, None)
        if item is self.sel_tree.topLevelItem(0):
            self.sel_hint.setText(f"{len(folders)} top-level folder(s) - expand to choose")

    def _sel_expand(self, item):
        if item.data(0, Qt.ItemDataRole.UserRole + 1):
            return  # already loaded
        rel = item.data(0, Qt.ItemDataRole.UserRole) or ""
        w = Worker(lambda: fetch_remote_folders(self._remote_root, rel))
        w.done.connect(lambda r, item=item: self._sel_populate(item, r))
        self._workers.append(w)
        w.start()

    def _derive_selection(self) -> list:
        root = self.sel_tree.topLevelItem(0)
        if root is None:
            return []
        if root.checkState(0) == Qt.CheckState.Checked:
            return []
        out = []

        def walk(item):
            for i in range(item.childCount()):
                c = item.child(i)
                rel = c.data(0, Qt.ItemDataRole.UserRole)
                if not rel:
                    continue
                if c.checkState(0) == Qt.CheckState.Checked:
                    out.append(rel)
                else:
                    walk(c)

        walk(root)
        return out

    # --- preference toggles

    def _autostart_toggled(self, checked: bool):
        exe = shutil.which("systemctl")
        if not exe:
            return
        ok = subprocess.run([exe, "--user", "enable" if checked else "disable",
                             JOURNAL_UNIT], capture_output=True, timeout=15).returncode == 0
        self.start_hint.setText("starts at login" if ok else "could not change autostart")

    # ------------------------------------------------------------------ account

    def load_identity(self):
        w = Worker(fetch_identity)
        w.done.connect(self._identity_loaded)
        self._workers.append(w)
        w.start()

    def _identity_loaded(self, result):
        if isinstance(result, Exception):
            return
        self._identity = result
        email = result.get("email")
        logged_in = result.get("logged_in")
        self.val_email.setText(email or (result.get("error") or "not logged in"))
        self.badge_login.setText("logged in" if logged_in else "not logged in")
        color = "#2e9e5b" if logged_in else "#d9822b"
        self.badge_login.setStyleSheet(f"background: {color}22; color: {color};")
        self.btn_login.setVisible(not logged_in)
        self.btn_logout.setVisible(logged_in)
        ver = result.get("version", "?")
        self.side_version.setText("CLI " + ver)
        self.about_version.setText(f"GUI {GUI_VERSION}  ·  CLI {ver}")
        self.refresh_details()

    def login_terminal(self):
        cli = resolve_cli()
        if not cli:
            QMessageBox.warning(self, "CLI not found", "The official CLI was not found.")
            return
        for name, args in (("konsole", ["-e"]), ("kitty", []), ("alacritty", ["-e"]),
                           ("xfce4-terminal", ["-e"]), ("gnome-terminal", ["--"]),
                           ("xterm", ["-e"])):
            exe = shutil.which(name)
            if exe:
                script = f'{shlex.quote(cli)} auth login; echo; echo "You can close this window."'
                subprocess.Popen([exe, *args, "sh", "-c", script])
                QTimer.singleShot(8000, self.load_identity)
                return
        QMessageBox.information(self, "No terminal found",
                                "Open a terminal and run:\n  proton-drive auth login")

    def logout(self):
        if QMessageBox.question(self, "Log out", "Log out of Proton Drive on this machine?") \
                != QMessageBox.StandardButton.Yes:
            return
        cli = resolve_cli()
        if cli:
            subprocess.run([cli, "auth", "logout"], capture_output=True, timeout=120)
        self.load_identity()

    # ------------------------------------------------------------------ notifications

    def _cool_ok(self, key: str, seconds: int = 30) -> bool:
        now = datetime.datetime.now().timestamp()
        if now - self._notify_cool.get(key, 0) < seconds:
            return False
        self._notify_cool[key] = now
        return True

    def _notify(self, title: str, msg: str):
        if self.tray:
            self.tray.showMessage(title, msg, QSystemTrayIcon.MessageIcon.Information, 4000)

    def _poll_notifications(self, health: dict):
        lines = journal_lines(60)
        for ln in lines:
            ts = journal_ts(ln)
            if ts is None or ts <= self._last_seen_ts:
                continue
            self._last_seen_ts = ts
            body = journal_body(ln)
            if "CONFLICT" in body and self.chk_note_conflicts.isChecked() \
                    and self._cool_ok("conflict"):
                self._notify("Proton Drive - conflict",
                             re.sub(r"^CONFLICT\s+", "", body)[:150])
            elif ("ERROR" in body or "FAILED" in body) and self.chk_note_errors.isChecked() \
                    and self._cool_ok("error"):
                self._notify("Proton Drive - sync error", body[:150])
        # finished round with real work done
        if (self._prev_health_status == "syncing" and health.get("status") == "ok"
                and self.chk_note_done.isChecked()
                and (health.get("uploaded", 0) or health.get("downloaded", 0))
                and self._cool_ok("done", 60)):
            self._notify("Proton Drive - synced",
                         f"{health.get('uploaded', 0)} uploaded, "
                         f"{health.get('downloaded', 0)} downloaded")

    # ------------------------------------------------------------------ window

    def quit_app(self):
        self._quitting = True
        if self.tray:
            self.tray.hide()
        QApplication.quit()

    def closeEvent(self, event: QCloseEvent):  # noqa: N802 - Qt naming
        if not self._quitting and self.tray and QSystemTrayIcon.isSystemTrayAvailable():
            event.ignore()
            self.hide()
            if not self._tray_warned:
                self._tray_warned = True
                self.tray.showMessage("Proton Drive",
                                      "Still running in the tray - right-click to quit.",
                                      QSystemTrayIcon.MessageIcon.Information, 3000)
            return
        event.accept()


# --------------------------------------------------------------------------- main


def main() -> int:
    smoke = "--smoke" in sys.argv
    if smoke:
        os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

    app = QApplication(sys.argv)
    app.setApplicationName("Proton Drive")
    app.setOrganizationName("ProtonDriveCLI")
    app.setStyle("Fusion")

    prefs = QSettings("ProtonDriveCLI", "ProtonDriveGUI")
    theme = prefs.value("theme", "system")
    MainWindow.apply_theme(theme if theme in ("system", "light", "dark") else "system")

    STATE_DIR.mkdir(parents=True, exist_ok=True)
    lock = QLockFile(str(LOCK_FILE))
    if not smoke and not lock.tryLock(100):
        QMessageBox.information(None, "Already running", "The Proton Drive app is already open.")
        return 0

    win = MainWindow()
    # preferences that act immediately
    win.chk_autostart.toggled.connect(win._autostart_toggled)
    win.chk_starttray.toggled.connect(
        lambda v: win._prefs.setValue("start_in_tray", v))
    for chk, key in ((win.chk_note_errors, "notify_errors"),
                     (win.chk_note_conflicts, "notify_conflicts"),
                     (win.chk_note_done, "notify_complete")):
        chk.toggled.connect(lambda v, k=key: win._prefs.setValue(k, v))

    if smoke or not prefs.value("start_in_tray", False, type=bool):
        win.show()
    else:
        win.tray and win.tray.showMessage(
            "Proton Drive", "Started in the tray - click the icon to open.",
            QSystemTrayIcon.MessageIcon.Information, 3000)

    if smoke:
        QTimer.singleShot(2500, app.quit)
    rc = app.exec()
    lock.unlock()
    return rc


if __name__ == "__main__":
    sys.exit(main())
