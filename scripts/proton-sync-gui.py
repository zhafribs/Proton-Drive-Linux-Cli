#!/usr/bin/env python3
"""
proton-sync-gui.py - "Proton Drive" desktop app for Linux (unofficial).

Part of Proton-Drive-Linux-Cli (MIT). A professional front-end for the
two-way folder sync service (proton-sync.py) on top of the OFFICIAL CLI:

* Dashboard  - live status, account, one-click Sync now / Open folder /
               Pause-Resume, recent activity
* Activity   - live service log with filter
* Settings   - edits ~/.config/proton-sync.conf and restarts the service
* About      - versions, links, license + legal notes
* tray icon  - status at a glance; closing the window keeps it running

No third-party dependencies beyond PyQt6 (official repos) and the official
Proton Drive CLI. Nothing here talks to Proton directly - it drives the same
service and CLI you would use by hand.

Run:  python3 scripts/proton-sync-gui.py      (or the proton-drive-gui shim)
"""

import datetime
import os
import re
import shlex
import shutil
import subprocess
import sys
from pathlib import Path

from PyQt6.QtCore import QLockFile, QUrl, Qt, QThread, QTimer, pyqtSignal as Signal
from PyQt6.QtGui import (QCloseEvent, QColor, QDesktopServices, QIcon, QPainter,
                         QPalette, QPen, QPixmap, QFont)
from PyQt6.QtSvg import QSvgRenderer
from PyQt6.QtWidgets import (QApplication, QCheckBox, QFileDialog, QFrame,
                             QHBoxLayout, QLabel, QLineEdit, QListWidget,
                             QListWidgetItem, QMessageBox, QPlainTextEdit,
                             QPushButton, QScrollArea, QSizePolicy, QSpinBox,
                             QStackedWidget, QStyle, QSystemTrayIcon, QMenu,
                             QVBoxLayout, QWidget)

GUI_VERSION = "1.0.0"

REPO_URL = "https://github.com/zhafribs/Proton-Drive-Linux-Cli"
CONF_FILE = Path.home() / ".config" / "proton-sync.conf"
UNIT_FILE = Path.home() / ".config" / "systemd" / "user" / "proton-sync.service"
STATE_DIR = Path(os.environ.get("XDG_STATE_HOME", str(Path.home() / ".local/state"))) / "proton-sync"
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
        if m:
            cand = m.group(1).strip()
            if os.access(cand, os.X_OK):
                return cand
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


def write_conf(values: dict):
    CONF_FILE.parent.mkdir(parents=True, exist_ok=True)
    header = ("# proton-sync settings - editable by hand or with the Proton Drive GUI\n"
              "# changes apply after the service restarts\n")
    body = "".join(f"{k}={values.get(k, v)}\n" for k, v in CONF_DEFAULTS.items())
    CONF_FILE.write_text(header + body, encoding="utf-8")


def systemctl(*args, timeout=15) -> bool:
    try:
        return subprocess.run(["systemctl", "--user", *args],
                              capture_output=True, timeout=timeout).returncode == 0
    except (subprocess.TimeoutExpired, FileNotFoundError):
        return False


def service_active() -> bool:
    try:
        return subprocess.run(["systemctl", "--user", "is-active", "--quiet", JOURNAL_UNIT],
                              timeout=10).returncode == 0
    except (subprocess.TimeoutExpired, FileNotFoundError):
        return False


ISO_RE = re.compile(r"^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}[+-]\d{4})")


def journal_lines(n=400) -> list:
    try:
        r = subprocess.run(["journalctl", "--user", "-u", JOURNAL_UNIT, "--no-pager",
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
    """Strip the 'HOST UNIT[pid]: ' prefix journald adds."""
    return line.split("]: ", 1)[-1] if "]: " in line else line


def analyse(lines, active) -> dict:
    """Work out what the service is doing right now."""
    last_cycle_ts, last_cycle_text = None, ""
    activity_ts, activity_text = None, ""
    last_error = ""
    for ln in lines:
        body = journal_body(ln)
        ts = journal_ts(ln)
        if "cycle done" in body:
            last_cycle_ts, last_cycle_text = ts, body.strip()
            last_error = ""
        elif any(k in body for k in ("UPLOAD ", "DOWNLOAD ", "MKDIR  ", "manual sync")):
            activity_ts, activity_text = ts, body.strip()
        elif "ERROR" in body or "FAILED" in body:
            last_error = body.strip()
    if not active:
        status = "paused"
    elif last_error and (last_cycle_ts is None or (journal_ts_of(last_error, lines) or last_cycle_ts) >= last_cycle_ts):
        status = "error"
    elif activity_ts and (last_cycle_ts is None or activity_ts > last_cycle_ts):
        status = "syncing"
    else:
        status = "synced"
    return {"status": status, "last_cycle_ts": last_cycle_ts,
            "last_cycle_text": last_cycle_text, "last_error": last_error,
            "activity": activity_text or last_cycle_text}


def journal_ts_of(text, lines):
    for ln in reversed(lines):
        if text in ln:
            return journal_ts(ln)
    return None


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


def svg_pixmap(path: str, size: int) -> QPixmap:
    pm = QPixmap(size, size)
    pm.fill(Qt.GlobalColor.transparent)
    if path and QSvgRenderer(path).isValid():
        QSvgRenderer(path).render(QPainter(pm))
    else:
        p = QPainter(pm)
        p.fillRect(0, 0, size, size, QColor(ACCENT))
        p.setPen(Qt.PenStyle.NoPen)
        p.end()
    return pm


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


# --------------------------------------------------------------------------- worker


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
        out["version"] = (r.stdout or r.stderr).strip().splitlines()[0] if (r.stdout or r.stderr).strip() else "?"
    except subprocess.TimeoutExpired:
        out["version"] = "?"
    try:
        r = subprocess.run([cli, "fs", "list", "/my-files", "--json"],
                           capture_output=True, text=True, timeout=120)
        if r.returncode == 0 and r.stdout.strip():
            import json
            for e in json.loads(r.stdout):
                email = (e.get("ownedBy") or {}).get("email")
                if email:
                    out["email"] = email
                    out["logged_in"] = True
                    break
    except Exception:  # noqa: BLE001
        pass
    return out


# --------------------------------------------------------------------------- styling

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
QLabel#cardTitle {{ color: {MUTED}; font-size: 11px; font-weight: 600;
                    text-transform: uppercase; }}
QLabel#cardValue {{ font-size: 20px; font-weight: 700; }}
QLabel#statusTitle {{ font-size: 20px; font-weight: 700; }}
QLabel#statusDetail, QLabel#hint {{ color: {MUTED}; }}
QLabel#badge {{ padding: 3px 10px; border-radius: 10px; font-size: 11px; font-weight: 600; }}
QLabel#legal {{ color: {MUTED}; font-size: 12px; }}
QPushButton {{ background: {CARD}; border: 1px solid {BORDER}; border-radius: 8px;
               padding: 7px 14px; font-weight: 600; }}
QPushButton:hover {{ background: {HOVER}; }}
QPushButton:disabled {{ color: {MUTED}; }}
QPushButton#primary {{ background: {ACCENT}; border: none; color: #ffffff; }}
QPushButton#primary:hover {{ background: #5b3ce0; }}
QPushButton#danger {{ color: #d04444; }}
QLineEdit, QSpinBox, QPlainTextEdit {{ background: {INPUT}; border: 1px solid {BORDER};
    border-radius: 8px; padding: 7px 10px; selection-background-color: {ACCENT}; }}
QPlainTextEdit {{ font-family: "JetBrains Mono", "Fira Code", monospace; font-size: 12px; }}
QLineEdit:focus, QSpinBox:focus, QPlainTextEdit:focus {{ border: 1px solid {ACCENT}; }}
QCheckBox {{ spacing: 8px; }}
QCheckBox::indicator {{ width: 17px; height: 17px; }}
QScrollArea {{ border: none; }}
"""


def build_qss(dark: bool) -> str:
    v = dict(DARK if dark else LIGHT)
    v["ACCENT"] = ACCENT
    return QSS.format(**v)


# --------------------------------------------------------------------------- widgets


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


# --------------------------------------------------------------------------- window


class MainWindow(QWidget):
    def __init__(self):
        super().__init__()
        self.setWindowTitle("Proton Drive")
        self.setMinimumSize(940, 640)
        self.resize(1020, 700)
        self._quitting = False
        self._tray_warned = False
        self._identity = {"version": "?", "email": None, "logged_in": False}
        self._status = {"status": "unknown", "last_cycle_ts": None, "activity": "", "last_error": ""}
        self._log_lines = []
        self._workers = []

        self._icon_svg = find_icon()
        self.setWindowIcon(QIcon(svg_pixmap(self._icon_svg, 64)))

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

        self.refresh_status()
        self.refresh_logs()
        self.refresh_counts()
        self.load_identity()

    # ------------------------------------------------------------------ UI build

    def _build_ui(self):
        root = QHBoxLayout(self)
        root.setContentsMargins(0, 0, 0, 0)
        root.setSpacing(0)

        # ---- sidebar
        side = QFrame()
        side.setObjectName("sidebar")
        side.setFixedWidth(216)
        sl = QVBoxLayout(side)
        sl.setContentsMargins(14, 18, 14, 16)
        sl.setSpacing(4)

        head = QHBoxLayout()
        head.setSpacing(10)
        glyph = QLabel()
        glyph.setPixmap(svg_pixmap(self._icon_svg, 30))
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
        self.nav.setFixedHeight(160)
        for label in ("Dashboard", "Activity", "Settings", "About"):
            QListWidgetItem(label).setFlags(Qt.ItemFlag.ItemIsEnabled | Qt.ItemFlag.ItemIsSelectable)
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

        # ---- pages
        self.stack = QStackedWidget()
        root.addWidget(self.stack, 1)
        self.stack.addWidget(self._dashboard_page())
        self.stack.addWidget(self._activity_page())
        self.stack.addWidget(self._settings_page())
        self.stack.addWidget(self._about_page())
        self.nav.setCurrentRow(0)

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

        # status banner
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
        self.btn_pause.clicked.connect(self.toggle_pause)
        for b in (self.btn_sync, self.btn_folder, self.btn_pause):
            btns.addWidget(b)
        bl.addLayout(btns)
        col.addWidget(banner)

        # stat cards
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

        # account
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

        # recent activity
        act, cl = card("Recent activity")
        self.preview = QPlainTextEdit()
        self.preview.setReadOnly(True)
        self.preview.document().setMaximumBlockCount(8)
        self.preview.setMinimumHeight(120)
        cl.addWidget(self.preview)
        col.addWidget(act)

        col.addStretch(1)
        sc = scroll_page(inner)
        outer.addWidget(sc, 1)
        return page

    def _activity_page(self) -> QWidget:
        page = QWidget()
        outer = QVBoxLayout(page)
        outer.setContentsMargins(26, 22, 26, 18)
        outer.setSpacing(14)
        outer.addWidget(page_header("Activity", "Live log of the sync service"))

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

        self.chk_inotify = QCheckBox("Upload local changes instantly (watch the folder)")
        form.addWidget(self.chk_inotify)
        self.chk_deletes = QCheckBox("Also sync deletions (files deleted on one side get"
                                     " trashed on the other side)")
        form.addWidget(self.chk_deletes)
        hint = QLabel("Off = a file deleted on one side stays safely on the other side.")
        hint.setObjectName("hint")
        form.addWidget(hint)

        row = QHBoxLayout()
        row.setSpacing(8)
        row.addWidget(QLabel("Parallel scan connections"))
        self.sp_workers = QSpinBox()
        self.sp_workers.setRange(1, 16)
        row.addWidget(self.sp_workers)
        row.addStretch(1)
        form.addLayout(row)
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
        self.load_settings()
        return page

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
        big.setPixmap(svg_pixmap(self._icon_svg, 56))
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
            "Everything it does, you could do by hand in a terminal - it just does it for you, "
            "with your settings from the Settings page.")
        how.setObjectName("legal")
        how.setWordWrap(True)
        lay.addWidget(how)
        col.addWidget(box)

        legal, ll = card("License and legal")
        txt = QLabel(
            "<b>Unofficial.</b> Not affiliated with, endorsed, sponsored or supported by Proton AG. "
            "This app talks to the official Proton Drive CLI (its payload is MIT, "
            "Copyright (c) 2025-2026 Proton AG; source: github.com/ProtonDriveApps/sdk). "
            "This repository's own scripts, including this GUI, are MIT (see LICENSE in the repo). "
            "'Proton' and 'Proton Drive' are trademarks of Proton AG, used here only to identify "
            "the product being packaged. No AUR, no third-party binaries: only official Proton "
            "downloads and official-repo packages.<br><br>"
            "Not legal advice.")
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
        self.tray = QSystemTrayIcon(tray_icon(svg_pixmap(self._icon_svg, 64), "unknown"), self)
        self.tray.setToolTip("Proton Drive")
        menu = QMenu()
        self.act_show = menu.addAction("Show window")
        self.act_show.triggered.connect(self.showNormal)
        menu.addSeparator()
        self.act_sync = menu.addAction("Sync now")
        self.act_sync.triggered.connect(self.sync_now)
        self.act_folder = menu.addAction("Open folder")
        self.act_folder.triggered.connect(self.open_folder)
        menu.addSeparator()
        self.act_pause = menu.addAction("Pause sync")
        self.act_pause.triggered.connect(self.toggle_pause)
        menu.addSeparator()
        act_quit = menu.addAction("Quit")
        act_quit.triggered.connect(self.quit_app)
        self.tray.setContextMenu(menu)
        self.tray.activated.connect(self._tray_activated)
        self.tray.show()

    def _tray_activated(self, reason):
        if reason == QSystemTrayIcon.ActivationReason.Trigger:
            self.showNormal()
            self.raise_()
            self.activateWindow()

    # ------------------------------------------------------------------ actions

    def _switch_page(self, row: int):
        self.stack.setCurrentIndex(max(0, row))

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

    def refresh_status(self):
        active = service_active()
        self._status = analyse(journal_lines(400), active)
        st = self._status["status"]
        color = STATUS_COLORS.get(st, STATUS_COLORS["unknown"])

        self.status_title.setText(STATUS_LABELS.get(st, st))
        if st == "error" and self._status.get("last_error"):
            detail = self._status["last_error"]
        elif st == "paused":
            detail = "Service is stopped. Press Resume to continue syncing."
        else:
            detail = self._status.get("activity") or ""
        self.status_detail.setText(detail[:200])
        self.status_title.setStyleSheet(f"color: {color};")

        self.side_dot.setStyleSheet(
            f"border-radius: 6px; background: {color}; min-width:12px; max-width:12px;"
            "min-height:12px; max-height:12px;")
        self.side_status.setText(STATUS_LABELS.get(st, st))

        self.val_service.setText("running" if active else "stopped")
        self.val_sync.setText(rel_time(self._status.get("last_cycle_ts")))

        self.btn_pause.setText(" Pause" if active else " Resume")
        self.btn_pause.setIcon(self.style().standardIcon(
            QStyle.StandardPixmap.SP_MediaPause if active else QStyle.StandardPixmap.SP_MediaPlay))
        self.btn_sync.setEnabled(active)
        self.act_sync.setEnabled(active)
        self.act_pause.setText("Pause sync" if active else "Resume sync")

        if self.tray:
            self.tray.setIcon(tray_icon(svg_pixmap(self._icon_svg, 64), st))
            self.tray.setToolTip(f"Proton Drive - {STATUS_LABELS.get(st, st)}")

    def refresh_logs(self):
        self._log_lines = journal_lines(400)
        self.preview.setPlainText("\n".join(self._log_lines[-8:]) or "(no activity yet)")
        if self.chk_auto.isChecked():
            self.render_logs()

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
        self.save_hint.setText("log copied")

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

    def sync_now(self):
        if not service_active():
            QMessageBox.information(self, "Sync paused",
                                    "The sync service is paused. Press Resume first.")
            return
        STATE_DIR.mkdir(parents=True, exist_ok=True)
        WAKE_FILE.touch()
        self.status_detail.setText("manual sync requested...")

    def open_folder(self):
        root = Path(self.ed_local.text().strip() or CONF_DEFAULTS["PROTON_SYNC_LOCAL"]).expanduser()
        root.mkdir(parents=True, exist_ok=True)
        QDesktopServices.openUrl(QUrl.fromLocalFile(str(root)))

    def toggle_pause(self):
        if service_active():
            systemctl("stop", JOURNAL_UNIT)
        else:
            systemctl("start", JOURNAL_UNIT)
        QTimer.singleShot(600, self.refresh_status)

    # ------------------------------------------------------------------ settings

    def load_settings(self):
        c = read_conf()
        self.ed_local.setText(c["PROTON_SYNC_LOCAL"])
        self.ed_remote.setText(c["PROTON_SYNC_REMOTE"])
        self.sp_interval.setValue(int(c["PROTON_SYNC_INTERVAL"]))
        self.chk_inotify.setChecked(c["PROTON_SYNC_INOTIFY"] == "1")
        self.chk_deletes.setChecked(c["PROTON_SYNC_DELETES"] == "1")
        self.sp_workers.setValue(int(c["PROTON_SYNC_WORKERS"]))
        self.save_hint.setText("")

    def save_settings(self):
        local = self.ed_local.text().strip()
        if not local.startswith("/"):
            QMessageBox.warning(self, "Invalid folder", "The local folder must be an absolute path.")
            return
        values = {
            "PROTON_SYNC_LOCAL": local,
            "PROTON_SYNC_REMOTE": self.ed_remote.text().strip() or "/my-files",
            "PROTON_SYNC_INTERVAL": str(self.sp_interval.value()),
            "PROTON_SYNC_DELETES": "1" if self.chk_deletes.isChecked() else "0",
            "PROTON_SYNC_INOTIFY": "1" if self.chk_inotify.isChecked() else "0",
            "PROTON_SYNC_WORKERS": str(self.sp_workers.value()),
        }
        answer = QMessageBox.question(
            self, "Save settings",
            "Save and restart the sync service now?\n\n"
            "(An upload or download in progress will simply continue in the next cycle.)")
        if answer != QMessageBox.StandardButton.Yes:
            return
        write_conf(values)
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

    def _browse(self):
        d = QFileDialog.getExistingDirectory(self, "Choose sync folder", self.ed_local.text())
        if d:
            self.ed_local.setText(d)

    # ------------------------------------------------------------------ account

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

    # ------------------------------------------------------------------ window

    def quit_app(self):
        self._quitting = True
        if self.tray:
            self.tray.hide()
        QApplication.quit()

    def closeEvent(self, event: QCloseEvent):  # noqa: N802 - Qt naming
        if (not self._quitting and self.tray
                and QSystemTrayIcon.isSystemTrayAvailable()):
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
    dark = app.palette().color(QPalette.ColorRole.Window).lightness() < 128
    app.setStyleSheet(build_qss(dark) + f"QPushButton#primary:hover {{ background: #5b3ce0; }}")

    STATE_DIR.mkdir(parents=True, exist_ok=True)
    lock = QLockFile(str(LOCK_FILE))
    if not smoke and not lock.tryLock(100):
        QMessageBox.information(None, "Already running", "The Proton Drive app is already open.")
        return 0

    win = MainWindow()
    win.show()
    if smoke:
        QTimer.singleShot(1500, app.quit)
    rc = app.exec()
    lock.unlock()
    return rc


if __name__ == "__main__":
    sys.exit(main())
