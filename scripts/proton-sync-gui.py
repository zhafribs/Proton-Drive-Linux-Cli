#!/usr/bin/env python3
"""
proton-sync-gui.py - "Proton Drive" desktop app for Linux (unofficial), v3.

Part of Proton-Drive-Linux-Cli (MIT). A friendly Qt6 desktop app for the
two-way folder sync (proton-sync.py) on top of the OFFICIAL Proton Drive CLI.

This v3 is designed for everyday users, not terminal people: a big friendly
status hero, plain language, one obvious button per job. All technical knobs
(service, retries, logs, paths) live behind "Advanced" and the About page.

The design takes ideas from how polished desktop cloud-drive apps look and
behave; the implementation is 100% original MIT-licensed code in this
repository - no code, assets or trademarks of other products are used.

Run:  python3 scripts/proton-sync-gui.py      (or the proton-drive-gui shim)
"""

import datetime
import functools
import hashlib
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import traceback
import urllib.request
from pathlib import Path

from PyQt6.QtCore import (QLockFile, QPoint, QSettings, QRectF, QUrl, Qt,
                          QThread, QTimer, pyqtSignal as Signal)
from PyQt6.QtGui import (QBrush, QCloseEvent, QColor, QDesktopServices,
                         QFont, QIcon, QLinearGradient, QPainter, QPalette,
                         QPen, QPixmap, QRadialGradient)
from PyQt6.QtSvg import QSvgRenderer
from PyQt6.QtWidgets import (QApplication, QCheckBox, QComboBox,
                             QGraphicsDropShadowEffect, QFileDialog, QFrame,
                             QHBoxLayout, QHeaderView, QLabel, QLineEdit,
                             QListWidget,
                             QMenu, QMessageBox, QPlainTextEdit, QPushButton,
                             QScrollArea, QSizePolicy, QSpinBox,
                             QStackedWidget, QStyle, QSystemTrayIcon,
                             QTreeWidget, QTreeWidgetItem, QVBoxLayout,
                             QWidget)
from PyQt6.QtGui import QFontMetrics

GUI_VERSION = "3.6.0"

REPO_URL = "https://github.com/zhafribs/Proton-Drive-Linux-Cli"
CONF_FILE = Path.home() / ".config" / "proton-sync.conf"
UNIT_FILE = Path.home() / ".config" / "systemd" / "user" / "proton-sync.service"
STATE_DIR = Path(os.environ.get("XDG_STATE_HOME", str(Path.home() / ".local/state"))) / "proton-sync"
HEALTH_FILE = STATE_DIR / "health.json"
WAKE_FILE = STATE_DIR / "wake"
LOCK_FILE = STATE_DIR / "gui.lock"
AUTOSTART_FILE = Path.home() / ".config" / "autostart" / "proton-drive.desktop"
JOURNAL_UNIT = "proton-sync"

CONF_DEFAULTS = {
    "PROTON_SYNC_LOCAL": str(Path.home() / "ProtonDrive"),
    "PROTON_SYNC_REMOTE": "/my-files",
    "PROTON_SYNC_INTERVAL": "300",
    "PROTON_SYNC_DELETES": "0",
    "PROTON_SYNC_INOTIFY": "1",
    "PROTON_SYNC_WORKERS": "12",
    "PROTON_SYNC_RETRIES": "3",
    "PROTON_SYNC_SUBFOLDERS": "",
    "PROTON_SYNC_EXCLUDE": "",
}

# friendly "check every ..." choices -> seconds
INTERVAL_CHOICES = [("Instant (30 seconds)", 30), ("Every minute", 60),
                    ("Every 5 minutes", 300), ("Every 15 minutes", 900),
                    ("Every 30 minutes", 1800), ("Every hour", 3600)]

ACCENT = "#6D4AFF"
ACCENT_LIGHT = "#8b6cff"
SUCCESS = "#16a34a"
WARNING = "#d97706"
ERROR = "#dc2626"
INFO = "#2563eb"
MUTED_GRAY = "#9ca3af"

# status -> (orb color, title, subtitle-template)
STATUS_TEXT = {
    "synced":    (SUCCESS, "You're up to date", ""),
    "syncing":   (ACCENT, "Syncing your files...", "Just a moment..."),
    "paused":    (WARNING, "Sync is paused",
                  "Nothing is syncing. Your files are safe."),
    "error":     (ERROR, "Couldn't sync", "{error}"),
    "unknown":   (MUTED_GRAY, "Checking your files...", ""),
}

# --------------------------------------------------------------------------- helpers


def resolve_cli() -> str:
    """The same binary the sync uses (from the unit file, else PATH)."""
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
    parts = ["# Proton Drive sync settings\n",
             "# (edited by the app - changes apply after a restart)\n"]
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


ISO_RE = re.compile(r"^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}[+-]\d{2}:?\d{2})")


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
        return ""
    now = datetime.datetime.now(dt.tzinfo)
    secs = max(0, int((now - dt).total_seconds()))
    if secs < 45:
        return "just now"
    if secs < 3600:
        return f"{max(1, secs // 60)} min ago"
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
    color = {"synced": SUCCESS, "syncing": ACCENT, "paused": WARNING,
             "error": ERROR}.get(status, MUTED_GRAY)
    pm = QPixmap(72, 72)
    pm.fill(Qt.GlobalColor.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.RenderHint.Antialiasing)
    p.drawPixmap(2, 2, base.scaled(66, 66, Qt.AspectRatioMode.KeepAspectRatio,
                                   Qt.TransformationMode.SmoothTransformation))
    p.setPen(QPen(QColor("#ffffff"), 4))
    p.setBrush(QColor(color))
    p.drawEllipse(52, 52, 18, 18)
    p.end()
    return QIcon(pm)


def desktop_notification(title: str, body: str = "") -> bool:
    """Show a desktop notification that carries our own icon.

    QSystemTrayIcon.showMessage(Information) maps to the freedesktop
    'idea' icon on Linux - a lightbulb in Breeze - which reads as 'tips'
    instead of our app, so talk to org.freedesktop.Notifications directly
    and hand it the cloud icon path.
    """
    exe = shutil.which("dbus-send")
    if not exe:
        return False
    try:
        r = subprocess.run(
            [exe, "--session", "--dest=org.freedesktop.Notifications",
             "--type=method_call", "/org/freedesktop/Notifications",
             "org.freedesktop.Notifications.Notify",
             "string:Proton Drive", "uint32:0", f"string:{find_icon()}",
             "string:Proton Drive", f"string:{body}",
             "array:string:", "dict:string:string:", "int32:15000"],
            capture_output=True, timeout=10)
        return r.returncode == 0
    except (OSError, subprocess.TimeoutExpired):
        return False


# --------------------------------------------------------------------------- workers


class Worker(QThread):
    """Run a slow function off the interface thread."""
    done = Signal(object)

    def __init__(self, fn):
        super().__init__()
        self.fn = fn

    def run(self):
        try:
            self.done.emit(self.fn())
        except Exception as e:  # noqa: BLE001
            self.done.emit(e)


def fetch_identity() -> dict:
    out = {"version": "?", "email": None, "logged_in": False}
    cli = resolve_cli()
    if not cli:
        out["error"] = "not found"
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


def fetch_remote_entries(remote_root: str, rel_path: str) -> list:
    """Everything inside REL_PATH (folders first) for the Cloud browser.

    Raises on CLI failure so the page can show an honest status line.
    """
    cli = resolve_cli()
    if not cli:
        raise RuntimeError("proton-drive CLI not found")
    rpath = "/".join([remote_root] + [c.replace("/", r"\/") for c in rel_path.split("/")]) \
        if rel_path else remote_root
    r = subprocess.run([cli, "fs", "list", rpath, "--json"],
                       capture_output=True, text=True, timeout=120)
    if r.returncode != 0:
        raise RuntimeError((r.stderr or r.stdout or "list failed").strip()[:200])
    try:
        raw = json.loads(r.stdout or "[]")
    except json.JSONDecodeError as e:
        raise RuntimeError(f"bad JSON from CLI: {e}") from e
    out = []
    for e in raw:
        if not isinstance(e, dict) or "type" not in e:
            continue                      # pseudo-sections (/, /trash, ...)
        name = (e.get("name") or {}).get("value")
        if not (e.get("name") or {}).get("ok", True) or not name:
            continue
        if re.match(r"^\.Trash-\d+$", name):
            continue
        is_dir = e.get("type") == "folder"
        media = e.get("mediaType") or ""
        out.append({"name": name,
                    "rel": f"{rel_path}/{name}" if rel_path else name,
                    "dir": is_dir,
                    "size": int(((e.get("activeRevision") or {}).get("claimedSize")) or 0),
                    "mtime": (e.get("modificationTime") or "").replace("T", " ")[:16],
                    "doc": (not is_dir) and media.startswith("application/vnd.proton")})
    out.sort(key=lambda x: (0 if x["dir"] else 1, x["name"].lower()))
    return out


# ------------------------------------------------ app install / autostart / updates

MENU_DIR = Path.home() / ".local" / "share" / "applications"
MENU_ENTRY = MENU_DIR / "proton-drive.desktop"
MENU_ICON_DIR = Path.home() / ".local" / "share" / "icons" / "hicolor" / "scalable" / "apps"
MENU_ICON = MENU_ICON_DIR / "proton-drive-gui.svg"
RELEASE_API = ("https://api.github.com/repos/zhafribs/"
               "Proton-Drive-Linux-Cli/releases/latest")
RELEASE_PAGE = "https://github.com/zhafribs/Proton-Drive-Linux-Cli/releases/latest"


def launch_command() -> str:
    """How this app is started from outside (login autostart, app menu)."""
    appimage = os.environ.get("APPIMAGE")
    if appimage and Path(appimage).is_file():
        return appimage
    shim = Path.home() / ".local" / "bin" / "proton-drive-gui"
    if shim.is_file():
        return str(shim)
    return f"{sys.executable} {Path(__file__).resolve()}"


def _desktop_exec(cmd: str) -> str:
    return f'"{cmd}"' if " " in cmd else cmd


def autostart_enabled() -> bool:
    return AUTOSTART_FILE.is_file()


def set_autostart(on: bool):
    if on:
        AUTOSTART_FILE.parent.mkdir(parents=True, exist_ok=True)
        AUTOSTART_FILE.write_text(
            "[Desktop Entry]\nType=Application\nName=Proton Drive\n"
            "Comment=Keep your Proton Drive folder in sync\n"
            f"Exec={_desktop_exec(launch_command())}\n"
            "X-GNOME-Autostart-enabled=true\n", encoding="utf-8")
    else:
        try:
            AUTOSTART_FILE.unlink()
        except OSError:
            pass


def menu_installed() -> bool:
    return MENU_ENTRY.is_file()


def set_menu_installed(on: bool):
    if on:
        MENU_DIR.mkdir(parents=True, exist_ok=True)
        MENU_ENTRY.write_text(
            "[Desktop Entry]\nType=Application\nName=Proton Drive\n"
            "Comment=Keep your Proton Drive folder in sync\n"
            f"Exec={_desktop_exec(launch_command())}\n"
            f"Icon={MENU_ICON}\nCategories=Utility;Network;\n"
            "Terminal=false\nStartupNotify=true\n", encoding="utf-8")
        icon = find_icon()
        if icon:
            MENU_ICON_DIR.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(icon, MENU_ICON)
    else:
        for p in (MENU_ENTRY, MENU_ICON):
            try:
                p.unlink()
            except OSError:
                pass
    exe = shutil.which("update-desktop-database")
    if exe:
        subprocess.run([exe, str(MENU_DIR)], capture_output=True, timeout=15)


def check_for_release() -> dict:
    """Latest published release of this project on GitHub.

    Tags are v<cli>-gui<gui> so every update gets its own release; the
    original single v<cli> tag is still understood as a GUI-less legacy.
    """
    req = urllib.request.Request(RELEASE_API,
                                 headers={"User-Agent": "proton-drive-gui"})
    with urllib.request.urlopen(req, timeout=20) as r:
        data = json.loads(r.read().decode())
    tag = data.get("tag_name") or ""
    core = tag.lstrip("v")
    m = re.fullmatch(r"(\d+\.\d+\.\d+)-gui(\d+\.\d+\.\d+)", core)
    cli, gui = (m.group(1), m.group(2)) if m else (core, "")
    return {"tag": tag, "version": core, "cli": cli, "gui": gui,
            "published": (data.get("published_at") or "")[:10],
            "assets": {a["name"]: a["browser_download_url"]
                       for a in data.get("assets", [])}}


def download_update(rel: dict, arch: str) -> Path:
    """Download the AppImage for this machine and verify its checksum."""
    if rel.get("gui"):
        name = f"ProtonDrive-CLI-{rel['cli']}-GUI-{rel['gui']}-{arch}.AppImage"
    else:  # legacy release tagged v<cli> without a GUI suffix
        name = f"ProtonDrive-CLI-{rel['cli']}-{arch}.AppImage"
    url = rel["assets"].get(name)
    sums = rel["assets"].get("SHA256SUMS")
    if not url or not sums:
        raise RuntimeError(f"no build for {arch} in release {rel['tag']}")
    dest = Path("/tmp") / f"proton-update-{name}"
    with urllib.request.urlopen(url, timeout=120) as r, open(dest, "wb") as f:
        shutil.copyfileobj(r, f)
    with urllib.request.urlopen(sums, timeout=30) as r:
        text = r.read().decode()
    want = None
    for line in text.splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[-1].lstrip("*") == name:
            want = parts[0]
            break
    if not want:
        dest.unlink(missing_ok=True)
        raise RuntimeError("checksum entry not found")
    if hashlib.sha256(dest.read_bytes()).hexdigest() != want:
        dest.unlink(missing_ok=True)
        raise RuntimeError("download corrupted (checksum mismatch)")
    dest.chmod(0o755)
    return dest


def install_update_and_restart(dl: Path):
    """Swap the running AppImage for the freshly downloaded one, then relaunch."""
    target = os.environ.get("APPIMAGE")
    if not target or not Path(target).is_file():
        raise RuntimeError("not running as an AppImage")
    os.replace(dl, target + ".new")
    os.replace(target, target + ".old")
    os.replace(target + ".new", target)
    subprocess.Popen([target], start_new_session=True,
                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


# --------------------------------------------------------------------------- design


PALETTES = {
    "light": dict(WINDOW="#f3f4fb", CARD="#ffffff", SIDE="#161129", SIDE_FG="#b9b2d6",
                  SIDE_ACTIVE_FG="#ffffff", TEXT="#14161f", MUTED="#6b7280",
                  INPUT="#f6f7fc", HOVER="#eceef6", SELECT="#ece7ff", BORDER="#e7e8f2"),
    "dark": dict(WINDOW="#0e1016", CARD="#171a23", SIDE="#0b0a14", SIDE_FG="#9d96bd",
                 SIDE_ACTIVE_FG="#ffffff", TEXT="#eceef4", MUTED="#9aa1ad",
                 INPUT="#1e2230", HOVER="#232733", SELECT="#262140", BORDER="#262b38"),
}

QSS = """
QWidget {{ font-size: 13.5px; color: {TEXT}; }}
QWidget#main {{ background: {WINDOW}; }}
QFrame#sidebar {{ background: {SIDE}; border: none; }}
QLabel#appName {{ font-size: 18px; font-weight: 700; color: #ffffff; letter-spacing: 0.2px; }}
QLabel#appSub {{ font-size: 11px; color: {SIDE_FG}; }}
QListWidget#nav {{ background: transparent; border: none; outline: none; font-size: 14px; }}
QListWidget#nav::item {{ padding: 11px 16px; margin: 3px 12px; border-radius: 11px;
                         color: {SIDE_FG}; }}
QListWidget#nav::item:hover {{ background: rgba(255,255,255,0.07); color: #ffffff; }}
QListWidget#nav::item:selected {{ background: qlineargradient(x1:0,y1:0,x2:1,y2:1,
                                    stop:0 {ACCENT_L}, stop:1 {ACCENT}); color: #ffffff; }}
QLabel#sideStatus {{ color: {SIDE_FG}; font-size: 12px; }}
QLabel#pageTitle {{ font-size: 26px; font-weight: 700; }}
QLabel#heroTitle {{ font-size: 27px; font-weight: 700; }}
QLabel#heroSub {{ font-size: 14px; color: {MUTED}; }}
QLabel#cardTitle {{ color: {MUTED}; font-size: 12px; font-weight: 700;
                    letter-spacing: 0.6px; }}
QLabel#h2 {{ font-size: 17px; font-weight: 700; }}
QLabel#muted {{ color: {MUTED}; }}
QLabel#small {{ font-size: 12px; color: {MUTED}; }}
QFrame#card {{ background: {CARD}; border-radius: 18px; }}
QLabel#pill {{ padding: 4px 12px; border-radius: 10px; font-size: 12px; font-weight: 700; }}
QPushButton {{ background: transparent; border: none; border-radius: 11px;
               padding: 9px 18px; font-weight: 600; font-size: 13.5px; }}
QPushButton:hover {{ background: {HOVER}; }}
QPushButton:disabled {{ color: {MUTED}; background: transparent; }}
QPushButton#primary {{ background: qlineargradient(x1:0,y1:0,x2:1,y2:1,
                        stop:0 {ACCENT_L}, stop:1 {ACCENT}); color: #ffffff;
                       border-radius: 22px; padding: 12px 22px; font-size: 14px; }}
QPushButton#primary:hover {{ background: qlineargradient(x1:0,y1:0,x2:1,y2:1,
                             stop:0 #9d84ff, stop:1 #7c5cff); }}
QPushButton#primary:disabled {{ background: {HOVER}; color: {MUTED}; }}
QPushButton#secondary {{ background: {CARD}; border: 1px solid {BORDER};
                         border-radius: 22px; padding: 11px 18px; font-size: 14px; }}
QPushButton#secondary:hover {{ background: {HOVER}; }}
QPushButton#ghostDanger {{ color: {ERROR}; }}
QPushButton#seg {{ border-radius: 10px; padding: 7px 16px; color: {MUTED}; }}
QPushButton#seg:checked {{ background: {CARD}; color: {TEXT}; }}
QLineEdit, QSpinBox, QPlainTextEdit, QComboBox, QTreeWidget {{
    background: {INPUT}; border: 1px solid transparent; border-radius: 12px;
    padding: 8px 12px; selection-background-color: {ACCENT}; font-size: 13.5px; }}
QLineEdit:focus, QSpinBox:focus, QTreeWidget:focus {{ border: 1px solid {ACCENT}; }}
QComboBox {{ padding: 9px 14px; }}
QComboBox::drop-down {{ border: none; width: 26px; }}
QComboBox QAbstractItemView {{ background: {CARD}; border: 1px solid {BORDER};
                               border-radius: 10px; padding: 6px;
                               selection-background-color: {SELECT};
                               selection-color: {TEXT}; }}
QPlainTextEdit {{ font-family: "JetBrains Mono", "Fira Code", monospace;
                  font-size: 12px; }}
QCheckBox {{ spacing: 11px; font-size: 13.5px; padding: 3px 0; }}
QCheckBox::indicator {{ width: 40px; height: 22px; }}
QCheckBox::indicator:unchecked {{ background: {BORDER}; border-radius: 11px; }}
QCheckBox::indicator:checked {{ background: qlineargradient(x1:0,y1:0,x2:1,y2:0,
                                 stop:0 {ACCENT_L}, stop:1 {ACCENT}); border-radius: 11px; }}
QTreeWidget {{ border: none; outline: none; }}
QTreeWidget::item {{ padding: 7px 4px; border-radius: 8px; }}
QTreeWidget::item:selected {{ background: {SELECT}; color: {TEXT}; }}
QHeaderView::section {{ background: transparent; border: none;
                        border-bottom: 1px solid {BORDER}; padding: 8px;
                        font-weight: 700; color: {MUTED}; font-size: 12px; }}
QScrollArea {{ border: none; }}
QScrollArea > QWidget {{ background: transparent; }}
QScrollArea > QWidget > QWidget {{ background: transparent; }}
QScrollBar:vertical {{ background: transparent; width: 10px; margin: 2px; }}
QScrollBar::handle:vertical {{ background: {BORDER}; border-radius: 5px;
                               min-height: 30px; }}
QScrollBar::handle:vertical:hover {{ background: {MUTED}; }}
QScrollBar::add-line, QScrollBar::sub-line {{ height: 0; }}
QMenu {{ background: {CARD}; border: 1px solid {BORDER}; border-radius: 12px;
         padding: 6px; }}
QMenu::item {{ padding: 9px 22px; border-radius: 8px; }}
QMenu::item:selected {{ background: {SELECT}; }}
QToolTip {{ background: {CARD}; color: {TEXT}; border: 1px solid {BORDER};
            padding: 6px; border-radius: 8px; }}
"""


def is_system_dark(app: QApplication) -> bool:
    c = app.palette().color(QPalette.ColorRole.Window)
    return (0.299 * c.red() + 0.587 * c.green() + 0.114 * c.blue()) < 128


def apply_theme(name: str):
    app = QApplication.instance()
    if not app:
        return
    dark = is_system_dark(app) if name == "system" else (name == "dark")
    p = dict(PALETTES["dark" if dark else "light"])
    p["ACCENT"] = ACCENT
    p["ACCENT_L"] = ACCENT_LIGHT
    p["ERROR"] = ERROR
    app.setStyleSheet(QSS.format(**p))


# --------------------------------------------------------------------------- widgets


def guard_slot(fn):
    """A crashing slot must never take the whole app down."""
    @functools.wraps(fn)
    def wrapper(self, *a, **k):
        try:
            return fn(self, *a, **k)
        except RuntimeError as e:
            if "has been deleted" in str(e):
                return  # window torn down with a timer tick still pending
            traceback.print_exc()
        except Exception:  # noqa: BLE001
            traceback.print_exc()
    return wrapper


class StatusOrb(QWidget):
    """The big round status icon on the home page (animated while syncing)."""

    def __init__(self, size=104):
        super().__init__()
        self.setFixedSize(size, size)
        self._color = QColor(MUTED_GRAY)
        self._status = "unknown"
        self._angle = 0
        self._timer = QTimer(self)
        self._timer.timeout.connect(self._spin)

    def set_status(self, status: str):
        color = STATUS_TEXT.get(status, (MUTED_GRAY, "", ""))[0]
        self._status = status
        self._color = QColor(color)
        if status == "syncing":
            if not self._timer.isActive():
                self._timer.start(70)
        else:
            self._timer.stop()
        self.update()

    def _spin(self):
        self._angle = (self._angle - 12) % 360
        self.update()

    def paintEvent(self, event):  # noqa: N802 - Qt naming
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        w = self.width()
        c = w / 2
        halo = QRadialGradient(c, c, c)
        halo.setColorAt(0.0, QColor(self._color))
        halo.setColorAt(0.72, QColor(self._color))
        halo.setColorAt(1.0, QColor(self._color).lighter(120))
        p.setBrush(QBrush(halo))
        p.setPen(Qt.PenStyle.NoPen)
        p.setOpacity(0.16)
        p.drawEllipse(0, 0, w, w)
        p.setOpacity(1.0)
        p.setBrush(QBrush(QColor(self._color)))
        p.drawEllipse(int(c * 0.42), int(c * 0.42), int(w * 0.58), int(w * 0.58))

        pen = QPen(QColor("#ffffff"))
        pen.setWidth(max(4, w // 16))
        pen.setCapStyle(Qt.PenCapStyle.RoundCap)
        p.setPen(pen)
        p.setBrush(Qt.BrushStyle.NoBrush)
        s = w * 0.28  # glyph half-size
        if self._status == "synced":
            p.drawPolyline([QPoint(int(c - s), int(c + s * 0.05)),
                            QPoint(int(c - s * 0.25), int(c + s * 0.7)),
                            QPoint(int(c + s), int(c - s * 0.6))])
        elif self._status == "syncing":
            rect_off = c * 0.62
            r = QRectF(rect_off, rect_off, w - 2 * rect_off, w - 2 * rect_off)
            p.drawArc(r, self._angle * 16, 80 * 16)
            p.setOpacity(0.45)
            p.drawArc(r, (self._angle + 180) * 16, 50 * 16)
        elif self._status == "paused":
            bw = max(4, w // 10)
            p.setPen(Qt.PenStyle.NoPen)
            p.drawRoundedRect(int(c - s * 0.7), int(c - s), bw, int(s * 2), bw // 2, bw // 2)
            p.drawRoundedRect(int(c + s * 0.7 - bw), int(c - s), bw, int(s * 2), bw // 2, bw // 2)
        elif self._status == "error":
            p.drawLine(int(c), int(c - s), int(c), int(c + s * 0.35))
            p.drawPoint(int(c), int(c + s * 0.8))
        else:  # unknown
            p.setOpacity(0.9)
            p.drawEllipse(int(c - s * 0.6), int(c - s * 0.6), int(s * 1.2), int(s * 1.2))
        p.end()


class Avatar(QWidget):
    """Round initial badge for the signed-in account."""

    def __init__(self, size=40):
        super().__init__()
        self.setFixedSize(size, size)
        self._letter = "?"

    def set_email(self, email):
        self._letter = (email or "?")[0].upper()
        self.update()

    def paintEvent(self, event):  # noqa: N802 - Qt naming
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        g = QLinearGradient(0, 0, self.width(), self.height())
        g.setColorAt(0, QColor(ACCENT_LIGHT))
        g.setColorAt(1, QColor(ACCENT))
        p.setBrush(g)
        p.setPen(Qt.PenStyle.NoPen)
        p.drawEllipse(0, 0, self.width(), self.height())
        p.setPen(QColor("#ffffff"))
        f = p.font()
        f.setBold(True)
        f.setPixelSize(int(self.width() * 0.44))
        p.setFont(f)
        p.drawText(self.rect(), Qt.AlignmentFlag.AlignCenter, self._letter)
        p.end()


def shadow_on(widget: QWidget, dark: bool):
    if dark:
        return
    sh = QGraphicsDropShadowEffect(widget)
    sh.setBlurRadius(34)
    sh.setOffset(0, 5)
    sh.setColor(QColor(30, 25, 70, 26))
    widget.setGraphicsEffect(sh)


def card(title: str = "", shadow: bool = True) -> tuple:
    frame = QFrame()
    frame.setObjectName("card")
    shadow_on(frame, is_system_dark(QApplication.instance())
              if QApplication.instance() else False)
    lay = QVBoxLayout(frame)
    lay.setContentsMargins(24, 20, 24, 20)
    lay.setSpacing(10)
    if title:
        t = QLabel(title.upper())
        t.setObjectName("cardTitle")
        lay.addWidget(t)
    return frame, lay


def section_label(text: str) -> QLabel:
    lab = QLabel(text)
    lab.setObjectName("cardTitle")
    return lab


def toggle_row(title: str, sub: str = "") -> tuple:
    """A settings row with an iOS-style switch. Returns (container, checkbox)."""
    box = QWidget()
    lay = QHBoxLayout(box)
    lay.setContentsMargins(0, 2, 0, 2)
    lay.setSpacing(12)
    col = QVBoxLayout()
    col.setSpacing(1)
    t = QLabel(title)
    col.addWidget(t)
    if sub:
        s = QLabel(sub)
        s.setObjectName("small")
        s.setWordWrap(True)
        col.addWidget(s)
    lay.addLayout(col, 1)
    chk = QCheckBox()
    lay.addWidget(chk)
    return box, chk


CHANGE_RE = re.compile(r"\b(UPLOAD|DOWNLOAD|MKDIR|TRASH|CONFLICT)\s*(.*)$")

DIR_STYLE = {"up": (SUCCESS, "\u2191"), "down": (INFO, "\u2193"),
             "warn": (WARNING, "!"), "other": (MUTED_GRAY, "\u00d7")}


def make_move_row(direction: str, name: str, path: str, action: str,
                  when: str) -> QWidget:
    """One 'file moved' row: colored arrow pill, name, path, action, time."""
    color, glyph = DIR_STYLE.get(direction, DIR_STYLE["other"])
    row = QWidget()
    lay = QHBoxLayout(row)
    lay.setContentsMargins(4, 7, 4, 7)
    lay.setSpacing(14)
    row.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Fixed)

    orb = QLabel(glyph)
    orb.setFixedSize(34, 34)
    orb.setAlignment(Qt.AlignmentFlag.AlignCenter)
    orb.setStyleSheet(f"background: {color}1f; color: {color}; border-radius: 17px;"
                      "font-weight: 700; font-size: 15px;")
    lay.addWidget(orb)

    col = QVBoxLayout()
    col.setSpacing(0)
    name_lab = QLabel(name)
    name_lab.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
    col.addWidget(name_lab)
    if path:
        sub = QLabel(path)
        sub.setObjectName("small")
        col.addWidget(sub)
    lay.addLayout(col, 1)

    right = QVBoxLayout()
    right.setSpacing(0)
    act = QLabel(action)
    act.setObjectName("small")
    act.setStyleSheet(f"color: {color}; font-weight: 600;")
    act.setAlignment(Qt.AlignmentFlag.AlignRight)
    right.addWidget(act)
    tm = QLabel(when)
    tm.setObjectName("small")
    tm.setAlignment(Qt.AlignmentFlag.AlignRight)
    right.addWidget(tm)
    lay.addLayout(right)
    return row


# --------------------------------------------------------------------------- window


class MainWindow(QWidget):
    def __init__(self):
        super().__init__()
        self.setWindowTitle("Proton Drive")
        # 920 is the narrowest window where every page fits without a
        # horizontal scrollbar and the About title/version lines stay single -
        # sidebar 216 + page margins 60 leave 644px of content against a
        # 607px worst-case page (Home), verified on x11 with room for font
        # differences across distros.
        self.setMinimumSize(920, 540)
        self.resize(940, 780)
        self._quitting = False
        self._tray_warned = False
        self._identity = {"version": "?", "email": None, "logged_in": False}
        self._log_lines = []
        self._changes = []
        self._workers = []
        self._prefs = QSettings("ProtonDriveCLI", "ProtonDriveGUI")
        self._apply_window_prefs()
        self._notify_cool = {}
        self._last_seen_ts = datetime.datetime.now().astimezone()
        self._prev_health_status = None
        self._selection = []
        self._remote_root = "/my-files"
        self._home_status = "unknown"
        self._sel_updating = False
        self._release = None
        self._logged_in = False

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
        self._counts_timer.timeout.connect(self.refresh_folder_stats)
        self._counts_timer.start(20000)

        self._tick_timer = QTimer(self)
        self._tick_timer.timeout.connect(self.update_countdown)
        self._tick_timer.start(1000)

        self.refresh_status()
        self.refresh_logs()
        self.refresh_folder_stats()
        self.load_identity()
        self.load_settings()

    # ------------------------------------------------------------------ UI build

    def _build_ui(self):
        root = QHBoxLayout(self)
        root.setContentsMargins(0, 0, 0, 0)
        root.setSpacing(0)
        self.setObjectName("main")

        side = QFrame()
        side.setObjectName("sidebar")
        side.setFixedWidth(216)
        sl = QVBoxLayout(side)
        sl.setContentsMargins(12, 26, 12, 20)
        sl.setSpacing(4)

        head = QHBoxLayout()
        head.setSpacing(11)
        glyph = QLabel()
        glyph.setPixmap(svg_pixmap(find_icon(), 34))
        head.addWidget(glyph)
        titles = QVBoxLayout()
        titles.setSpacing(0)
        name = QLabel("Proton Drive")
        name.setObjectName("appName")
        sub = QLabel("for Linux")
        sub.setObjectName("appSub")
        titles.addWidget(name)
        titles.addWidget(sub)
        head.addLayout(titles)
        head.addStretch(1)
        sl.addLayout(head)
        sl.addSpacing(26)

        self.nav = QListWidget()
        self.nav.setObjectName("nav")
        # the Qt list default size hint is a fixed 192px (fits 4 items only);
        # _nav_fit_items gives it real room per item so nothing clips/scrolls
        self.nav.setVerticalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        # natural height: a hardcoded one made the 4 items overflow into a
        # scrollbar; the stretch below absorbs the remaining sidebar space.
        for label in ("Home", "Cloud", "Activity", "Settings", "About"):
            self.nav.addItem(label)
        self.nav.currentRowChanged.connect(self._switch_page)
        sl.addWidget(self.nav)
        QTimer.singleShot(0, self._nav_fit_items)
        sl.addStretch(1)

        foot = QHBoxLayout()
        foot.setSpacing(9)
        self.side_dot = QLabel()
        self.side_dot.setFixedSize(10, 10)
        self.side_dot.setStyleSheet("background: %s; border-radius: 5px;" % MUTED_GRAY)
        foot.addWidget(self.side_dot)
        self.side_status = QLabel("Checking...")
        self.side_status.setObjectName("sideStatus")
        foot.addWidget(self.side_status)
        foot.addStretch(1)
        sl.addLayout(foot)
        unofficial = QLabel("Unofficial community app")
        unofficial.setObjectName("appSub")
        unofficial.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        sl.addWidget(unofficial)

        root.addWidget(side)

        self.stack = QStackedWidget()
        root.addWidget(self.stack, 1)
        self.stack.addWidget(self._home_page())
        self.stack.addWidget(self._cloud_page())
        self.stack.addWidget(self._activity_page())
        self.stack.addWidget(self._settings_page())
        self.stack.addWidget(self._about_page())
        self.nav.setCurrentRow(0)

    # ---- home ---------------------------------------------------------------

    def _home_page(self) -> QWidget:
        page = QWidget()
        outer = QVBoxLayout(page)
        outer.setContentsMargins(30, 30, 30, 20)
        outer.setSpacing(18)

        inner = QWidget()
        col = QVBoxLayout(inner)
        col.setContentsMargins(0, 0, 8, 0)
        col.setSpacing(18)

        # --- hero
        hero = QFrame()
        hero_l = QHBoxLayout(hero)
        hero_l.setContentsMargins(0, 0, 0, 0)
        hero_l.setSpacing(26)
        self.orb = StatusOrb()
        hero_l.addWidget(self.orb, 0, Qt.AlignmentFlag.AlignTop)
        textcol = QVBoxLayout()
        textcol.setSpacing(6)
        self.hero_title = QLabel("Checking your files...")
        self.hero_title.setObjectName("heroTitle")
        self.hero_sub = QLabel("")
        self.hero_sub.setObjectName("heroSub")
        self.hero_sub.setWordWrap(True)
        textcol.addWidget(self.hero_title)
        textcol.addWidget(self.hero_sub)

        btns = QHBoxLayout()
        btns.setSpacing(12)
        self.btn_open = QPushButton("  Open my folder")
        self.btn_open.setIcon(self.style().standardIcon(QStyle.StandardPixmap.SP_DirOpenIcon))
        self.btn_open.setObjectName("primary")
        self.btn_open.clicked.connect(self.open_folder)
        self.btn_sync = QPushButton("  Sync now")
        self.btn_sync.setIcon(self.style().standardIcon(QStyle.StandardPixmap.SP_BrowserReload))
        self.btn_sync.setObjectName("secondary")
        self.btn_sync.clicked.connect(self.sync_now)
        self.btn_pause = QPushButton("Pause")
        self.btn_pause.setIcon(self.style().standardIcon(QStyle.StandardPixmap.SP_MediaPause))
        self.btn_pause.setObjectName("secondary")
        self.btn_pause.clicked.connect(self._pause_clicked)
        btns.addWidget(self.btn_open)
        btns.addWidget(self.btn_sync)
        btns.addWidget(self.btn_pause)
        btns.addStretch(1)
        textcol.addLayout(btns)
        textcol.addStretch(1)
        hero_l.addLayout(textcol, 1)
        col.addWidget(hero)

        # --- account
        acct, al = card()
        al.setSpacing(0)
        arow = QHBoxLayout()
        arow.setSpacing(14)
        self.avatar = Avatar(44)
        arow.addWidget(self.avatar)
        acol = QVBoxLayout()
        acol.setSpacing(1)
        self.val_email = QLabel("Checking your account...")
        self.val_email.setObjectName("h2")
        self.badge_login = QLabel(" ")
        self.badge_login.setObjectName("small")
        acol.addWidget(self.val_email)
        acol.addWidget(self.badge_login)
        arow.addLayout(acol, 1)
        self.btn_login = QPushButton("Sign in")
        self.btn_login.setObjectName("secondary")
        self.btn_login.clicked.connect(self.login_terminal)
        self.btn_logout = QPushButton("Sign out")
        self.btn_logout.setObjectName("ghostDanger")
        self.btn_logout.clicked.connect(self.logout)
        self.btn_logout.setVisible(False)
        arow.addWidget(self.btn_login)
        arow.addWidget(self.btn_logout)
        al.addLayout(arow)
        col.addWidget(acct)

        # --- folder
        folder, fl = card("Your folder on this computer")
        frow = QHBoxLayout()
        frow.setSpacing(16)
        ficon = QLabel()
        ficon.setPixmap(self.style().standardIcon(
            QStyle.StandardPixmap.SP_DirOpenIcon).pixmap(30, 30))
        frow.addWidget(ficon)
        fcol = QVBoxLayout()
        fcol.setSpacing(1)
        self.folder_name = QLabel("ProtonDrive")
        self.folder_name.setObjectName("h2")
        self.val_path = QLabel("")
        self.val_path.setObjectName("small")
        fcol.addWidget(self.folder_name)
        fcol.addWidget(self.val_path)
        frow.addLayout(fcol, 1)
        self.val_stats = QLabel("")
        self.val_stats.setObjectName("muted")
        frow.addWidget(self.val_stats)
        fl.addLayout(frow)
        col.addWidget(folder)

        # --- recent
        recent, rl = card("Recently synced")
        rl.setSpacing(2)
        self.recent_box = QVBoxLayout()
        self.recent_box.setSpacing(0)
        rl.addLayout(self.recent_box)
        self.recent_empty = QLabel("Files you change will show up here.")
        self.recent_empty.setObjectName("muted")
        rl.addWidget(self.recent_empty)
        col.addWidget(recent)

        col.addStretch(1)
        outer.addWidget(scroll_page(inner), 1)
        return page

    # ---- activity -----------------------------------------------------------

    # ---- cloud ----------------------------------------------------------------

    def _cloud_page(self) -> QWidget:
        page = QWidget()
        outer = QVBoxLayout(page)
        outer.setContentsMargins(30, 30, 30, 20)
        outer.setSpacing(14)

        top = QHBoxLayout()
        top.setSpacing(12)
        self.cloud_back = QPushButton("←")
        self.cloud_back.setFixedWidth(44)
        self.cloud_back.setToolTip("Go back")
        self.cloud_back.setEnabled(False)
        self.cloud_back.clicked.connect(self._cloud_back)
        top.addWidget(self.cloud_back)
        self.cloud_path = QLabel("")
        self.cloud_path.setObjectName("small")
        self.cloud_path.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Fixed)
        top.addWidget(self.cloud_path, 1)
        btn_refresh = QPushButton("Refresh")
        btn_refresh.clicked.connect(lambda: self._cloud_load(self._cloud_rel, push=False))
        top.addWidget(btn_refresh)
        outer.addLayout(top)

        self.cloud_list = QTreeWidget()
        self.cloud_list.setHeaderLabels(["Name", "Size", "Modified"])
        self.cloud_list.setRootIsDecorated(False)
        self.cloud_list.setAlternatingRowColors(True)
        self.cloud_list.setUniformRowHeights(True)
        hdr = self.cloud_list.header()
        hdr.setStretchLastSection(False)
        hdr.setSectionResizeMode(0, QHeaderView.ResizeMode.Stretch)
        hdr.setSectionResizeMode(1, QHeaderView.ResizeMode.Fixed)
        hdr.setSectionResizeMode(2, QHeaderView.ResizeMode.Fixed)
        hdr.resizeSection(1, 96)
        hdr.resizeSection(2, 132)
        outer.addWidget(self.cloud_list, 1)

        bottom = QHBoxLayout()
        bottom.setSpacing(12)
        self.cloud_status = QLabel("")
        self.cloud_status.setObjectName("small")
        bottom.addWidget(self.cloud_status, 1)
        self.cloud_dl = QPushButton("  Download")
        self.cloud_dl.setEnabled(False)
        self.cloud_dl.clicked.connect(self._cloud_download)
        bottom.addWidget(self.cloud_dl)
        outer.addLayout(bottom)

        self._cloud_rel = ""
        self._cloud_hist: list = []
        self.cloud_list.itemDoubleClicked.connect(self._cloud_open_item)
        self.cloud_list.itemSelectionChanged.connect(
            lambda: self.cloud_dl.setEnabled(self.cloud_list.currentItem() is not None))
        return page

    def _cloud_shown(self):
        # re-list on every visit: fresh, and self-heals after an error
        self._cloud_load(self._cloud_rel, push=False)

    def _cloud_load(self, rel: str, push: bool = True):
        if push and rel != self._cloud_rel:
            self._cloud_hist.append(self._cloud_rel)
        self._cloud_rel = rel
        disp = f"/{rel}" if rel else "My Files"
        self.cloud_path.setToolTip(disp)
        self._cloud_paint_path()
        self.cloud_back.setEnabled(bool(self._cloud_hist))
        self.cloud_list.setEnabled(False)
        self.cloud_dl.setEnabled(False)
        self.cloud_status.setText("Opening...")
        w = Worker(lambda: fetch_remote_entries(self._remote_root or "/my-files", rel))
        w.done.connect(self._cloud_loaded)
        self._workers.append(w)
        w.start()

    def _cloud_paint_path(self):
        disp = f"/{self._cloud_rel}" if self._cloud_rel else "My Files"
        w = self.cloud_path.width()
        if w > 8:
            disp = QFontMetrics(self.cloud_path.font()).elidedText(
                disp, Qt.TextElideMode.ElideMiddle, w)
        self.cloud_path.setText(disp)

    def resizeEvent(self, event):
        super().resizeEvent(event)
        if getattr(self, "cloud_path", None) is not None:
            self._cloud_paint_path()

    @guard_slot
    def _cloud_loaded(self, result):
        self.cloud_list.setEnabled(True)
        self.cloud_list.clear()
        if isinstance(result, Exception):
            self.cloud_status.setText("Couldn't open this folder.")
            self.cloud_dl.setEnabled(False)
            return
        folder_icon = self.style().standardIcon(QStyle.StandardPixmap.SP_DirIcon)
        file_icon = self.style().standardIcon(QStyle.StandardPixmap.SP_FileIcon)
        for e in result:
            item = QTreeWidgetItem([e["name"],
                                    "" if e["dir"] else self._fmt_size(e["size"]),
                                    e["mtime"] or ""])
            item.setIcon(0, folder_icon if e["dir"] else file_icon)
            item.setData(0, Qt.ItemDataRole.UserRole, e["rel"])
            item.setData(0, Qt.ItemDataRole.UserRole + 1, e["dir"])
            item.setData(0, Qt.ItemDataRole.UserRole + 2, e["doc"])
            item.setData(0, Qt.ItemDataRole.UserRole + 3, e["size"])
            self.cloud_list.addTopLevelItem(item)
        self.cloud_status.setText("This folder is empty." if not result else "")

    @staticmethod
    def _fmt_size(n: int) -> str:
        if n <= 0:
            return ""
        if n < 1024:
            return f"{n} B"
        if n < 1024 ** 2:
            return f"{n / 1024:.0f} KB"
        if n < 1024 ** 3:
            return f"{n / 1024 ** 2:.1f} MB"
        return f"{n / 1024 ** 3:.1f} GB"

    def _cloud_open_item(self, item, _col):
        if item.data(0, Qt.ItemDataRole.UserRole + 1):
            self._cloud_load(item.data(0, Qt.ItemDataRole.UserRole))
        else:
            self._cloud_download()

    def _cloud_back(self):
        if self._cloud_hist:
            prev = self._cloud_hist.pop()
            self._cloud_load(prev, push=False)

    @guard_slot
    def _cloud_download(self):
        item = self.cloud_list.currentItem()
        if item is None or item.data(0, Qt.ItemDataRole.UserRole + 1):
            return
        if item.data(0, Qt.ItemDataRole.UserRole + 2):
            self.cloud_status.setText("Proton documents can't be downloaded.")
            return
        rel = item.data(0, Qt.ItemDataRole.UserRole)
        name = rel.rsplit("/", 1)[-1]
        start = Path.home() / "Downloads"
        if not start.is_dir():
            start = Path.home()
        chosen, _ = QFileDialog.getSaveFileName(self, "Save file", str(start / name))
        if not chosen:
            return
        self.cloud_dl.setEnabled(False)
        self.cloud_status.setText("Downloading...")
        root = self._remote_root or "/my-files"
        rpath = "/".join([root] + [c.replace("/", r"\/") for c in rel.split("/")])
        w = Worker(lambda: self._cloud_fetch(rpath, Path(chosen)))
        w.done.connect(lambda r, chosen=chosen: self._cloud_downloaded(r, chosen))
        self._workers.append(w)
        w.start()

    @staticmethod
    def _cloud_fetch(rpath: str, dest: Path) -> str:
        cli = resolve_cli()
        if not cli:
            raise RuntimeError("proton-drive CLI not found")
        tmp = Path(tempfile.mkdtemp(prefix="proton-cloud-"))
        r = subprocess.run([cli, "fs", "download", rpath, str(tmp)],
                           capture_output=True, text=True, timeout=600)
        if r.returncode != 0:
            shutil.rmtree(tmp, ignore_errors=True)
            raise RuntimeError((r.stderr or r.stdout or "download failed").strip()[:200])
        produced = tmp / dest.name
        if not produced.exists():
            files = [p for p in tmp.iterdir() if p.is_file()]
            if len(files) != 1:
                shutil.rmtree(tmp, ignore_errors=True)
                raise RuntimeError("downloaded file not found")
            produced = files[0]
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.move(str(produced), str(dest))
        shutil.rmtree(tmp, ignore_errors=True)
        return str(dest)

    @guard_slot
    def _cloud_downloaded(self, result, dest: str):
        self.cloud_dl.setEnabled(self.cloud_list.currentItem() is not None)
        if isinstance(result, Exception):
            self.cloud_status.setText("Download failed.")
            return
        self.cloud_status.setText("Saved.")
        desktop_notification("Downloaded", Path(dest).name)

    # ---- activity ---------------------------------------------------------------

    def _activity_page(self) -> QWidget:
        page = QWidget()
        outer = QVBoxLayout(page)
        outer.setContentsMargins(30, 30, 30, 20)
        outer.setSpacing(18)

        head = QHBoxLayout()
        title = QLabel("Activity")
        title.setObjectName("pageTitle")
        head.addWidget(title)
        head.addStretch(1)
        segbox = QFrame()
        segbox.setStyleSheet("background: rgba(127,127,127,0.12); border-radius: 12px;")
        seg = QHBoxLayout(segbox)
        seg.setContentsMargins(5, 5, 5, 5)
        seg.setSpacing(2)
        self.seg_files = QPushButton("Files moved")
        self.seg_files.setCheckable(True)
        self.seg_files.setChecked(True)
        self.seg_files.setObjectName("seg")
        self.seg_files.setAutoExclusive(True)
        self.seg_files.clicked.connect(lambda: self._seg_switch(0))
        self.seg_log = QPushButton("Technical log")
        self.seg_log.setCheckable(True)
        self.seg_log.setObjectName("seg")
        self.seg_log.setAutoExclusive(True)
        self.seg_log.clicked.connect(lambda: self._seg_switch(1))
        seg.addWidget(self.seg_files)
        seg.addWidget(self.seg_log)
        head.addWidget(segbox)
        outer.addLayout(head)

    # --- tab 1: friendly list
        self.tab_files = QWidget()
        tf = QVBoxLayout(self.tab_files)
        tf.setContentsMargins(0, 0, 0, 0)
        tf.setSpacing(12)
        bar = QHBoxLayout()
        bar.setSpacing(10)
        self.ch_filter = QLineEdit()
        self.ch_filter.setPlaceholderText("Search file names...")
        self.ch_filter.textChanged.connect(self.render_changes)
        bar.addWidget(self.ch_filter, 1)
        self.ch_kind = QComboBox()
        self.ch_kind.addItems(["Everything", "Uploaded", "Downloaded", "Other"])
        self.ch_kind.currentIndexChanged.connect(self.render_changes)
        bar.addWidget(self.ch_kind)
        self.ch_count = QLabel("")
        self.ch_count.setObjectName("small")
        bar.addWidget(self.ch_count)
        tf.addLayout(bar)

        scroll_files = QScrollArea()
        scroll_files.setWidgetResizable(True)
        scroll_files.setFrameShape(QFrame.Shape.NoFrame)
        scroll_files.viewport().setAutoFillBackground(False)
        self.moves_box = QVBoxLayout()
        self.moves_box.setSpacing(0)
        moves_wrap = QWidget()
        moves_wrap.setLayout(self.moves_box)
        scroll_files.setWidget(moves_wrap)
        tf.addWidget(scroll_files, 1)
        self.moves_empty = QLabel("Nothing has moved yet - change a file and it will "
                                  "appear here.")
        self.moves_empty.setObjectName("muted")
        tf.addWidget(self.moves_empty)

    # --- tab 2: technical log
        self.tab_log = QWidget()
        tl = QVBoxLayout(self.tab_log)
        tl.setContentsMargins(0, 0, 0, 0)
        tl.setSpacing(12)
        bar2 = QHBoxLayout()
        self.log_filter = QLineEdit()
        self.log_filter.setPlaceholderText("Filter log lines...")
        self.log_filter.textChanged.connect(self.render_logs)
        bar2.addWidget(self.log_filter, 1)
        btn_copy = QPushButton("Copy log")
        btn_copy.clicked.connect(self.copy_logs)
        bar2.addWidget(btn_copy)
        tl.addLayout(bar2)
        self.log_view = QPlainTextEdit()
        self.log_view.setReadOnly(True)
        self.log_view.document().setMaximumBlockCount(600)
        tl.addWidget(self.log_view, 1)

        self.act_stack = QStackedWidget()
        self.act_stack.addWidget(self.tab_files)
        self.act_stack.addWidget(self.tab_log)
        outer.addWidget(self.act_stack, 1)
        return page

    def _seg_switch(self, idx: int):
        self.act_stack.setCurrentIndex(idx)

    # ---- settings -----------------------------------------------------------

    def _settings_page(self) -> QWidget:
        page = QWidget()
        outer = QVBoxLayout(page)
        outer.setContentsMargins(30, 30, 30, 20)
        outer.setSpacing(18)
        title = QLabel("Settings")
        title.setObjectName("pageTitle")
        outer.addWidget(title)

        inner = QWidget()
        col = QVBoxLayout(inner)
        col.setContentsMargins(0, 0, 8, 0)
        col.setSpacing(16)

        # --- folder
        box, form = card("Where your files live")
        row = QHBoxLayout()
        row.setSpacing(14)
        ficon = QLabel()
        ficon.setPixmap(self.style().standardIcon(
            QStyle.StandardPixmap.SP_DirOpenIcon).pixmap(28, 28))
        row.addWidget(ficon)
        fcol = QVBoxLayout()
        fcol.setSpacing(0)
        self.ed_local = QLineEdit()
        self.ed_local.setReadOnly(True)
        self.ed_local.setCursorPosition(0)
        fcol.addWidget(self.ed_local)
        folder_hint = QLabel("Only this folder is synced.")
        folder_hint.setObjectName("small")
        fcol.addWidget(folder_hint)
        row.addLayout(fcol, 1)
        btn_browse = QPushButton("Choose folder...")
        btn_browse.clicked.connect(self._browse)
        row.addWidget(btn_browse)
        form.addLayout(row)
        col.addWidget(box)

        # --- what to sync
        box, form = card("What gets synced")
        form.setSpacing(14)
        self.chk_selective = QCheckBox("Only sync the folders I pick (instead of "
                                       "everything)")
        form.addWidget(self.chk_selective)
        self.sel_tree = QTreeWidget()
        self.sel_tree.setHeaderHidden(True)
        self.sel_tree.setMinimumHeight(160)
        self.sel_tree.itemExpanded.connect(self._sel_expand)
        self.sel_tree.itemChanged.connect(self._sel_item_changed)
        form.addWidget(self.sel_tree)
        row = QHBoxLayout()
        btn_reload = QPushButton("Refresh folder list")
        btn_reload.clicked.connect(self._sel_load_root)
        row.addWidget(btn_reload)
        self.sel_hint = QLabel("")
        self.sel_hint.setObjectName("small")
        row.addWidget(self.sel_hint, 1)
        form.addLayout(row)

        form.addSpacing(6)
        form.addWidget(section_label("NEVER SYNC FILES MATCHING"))
        self.ed_exclude = QLineEdit()
        self.ed_exclude.setPlaceholderText("For example:  *.tmp, .cache, *.iso")
        form.addWidget(self.ed_exclude)
        col.addWidget(box)

        # --- how often
        box, form = card("How often")
        form.setSpacing(12)
        row_t, self.chk_inotify = toggle_row("Upload my changes right away")
        form.addWidget(row_t)
        form.addSpacing(4)
        row = QHBoxLayout()
        row.setSpacing(10)
        lab = QLabel("Look for changes in the cloud")
        row.addWidget(lab)
        self.cmb_interval = QComboBox()
        for label, _secs in INTERVAL_CHOICES:
            self.cmb_interval.addItem(label)
        row.addWidget(self.cmb_interval)
        row.addStretch(1)
        form.addLayout(row)
        row_d, self.chk_deletes = toggle_row(
            "Also remove files I delete", "Both sides stay equal.")
        form.addWidget(row_d)
        col.addWidget(box)

        # --- notifications
        box, form = card("Notifications")
        form.setSpacing(6)
        self.chk_note_errors = QCheckBox("Tell me if something fails to sync")
        self.chk_note_conflicts = QCheckBox("Tell me if a file was edited in two "
                                            "places at once")
        self.chk_note_done = QCheckBox("Tell me when syncing finished")
        form.addWidget(self.chk_note_errors)
        form.addWidget(self.chk_note_conflicts)
        form.addWidget(self.chk_note_done)
        row = QHBoxLayout()
        btn_test = QPushButton("Send a test notification")
        btn_test.clicked.connect(
            lambda: self._notify("Proton Drive",
                                 "This is what notifications look like."))
        row.addWidget(btn_test)
        row.addStretch(1)
        form.addLayout(row)
        col.addWidget(box)

        # --- running in the background
        box, form = card("Running in the background")
        form.setSpacing(6)
        row_o, self.chk_top = toggle_row("Keep this window above other windows")
        form.addWidget(row_o)
        row_b, self.chk_close_tray = toggle_row(
            "Keep running when I close the window")
        form.addWidget(row_b)
        row_s, self.chk_starttray = toggle_row("Start quietly in the tray")
        form.addWidget(row_s)
        row_a, self.chk_autostart = toggle_row("Start syncing when I log in")
        form.addWidget(row_a)
        row_la, self.chk_appstart = toggle_row("Open Proton Drive when I log in")
        form.addWidget(row_la)
        col.addWidget(box)

        # --- appearance
        box, form = card("Appearance")
        row = QHBoxLayout()
        row.setSpacing(10)
        row.addWidget(QLabel("Theme"))
        self.cmb_theme = QComboBox()
        self.cmb_theme.addItems(["Match my system", "Light", "Dark"])
        self.cmb_theme.currentIndexChanged.connect(self._theme_changed)
        row.addWidget(self.cmb_theme)
        row.addStretch(1)
        form.addLayout(row)
        col.addWidget(box)

        # --- updates
        box, form = card("Updates")
        form.setSpacing(8)
        row_u, self.chk_updates = toggle_row("Check for new versions automatically")
        form.addWidget(row_u)
        self.lbl_update = QLabel("")
        self.lbl_update.setObjectName("small")
        self.lbl_update.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        form.addWidget(self.lbl_update)
        row = QHBoxLayout()
        row.setSpacing(8)
        btn_check = QPushButton("Check now")
        btn_check.clicked.connect(self._check_updates_clicked)
        row.addWidget(btn_check)
        self.btn_dl_update = QPushButton("Download and restart")
        self.btn_dl_update.setObjectName("primary")
        self.btn_dl_update.setVisible(False)
        self.btn_dl_update.clicked.connect(self._download_update)
        row.addWidget(self.btn_dl_update)
        btn_page = QPushButton("Open download page")
        btn_page.clicked.connect(
            lambda: QDesktopServices.openUrl(QUrl(RELEASE_PAGE)))
        row.addWidget(btn_page)
        row.addStretch(1)
        form.addLayout(row)
        col.addWidget(box)

        # --- installing on linux
        box, form = card("Installing on Linux")
        form.setSpacing(8)
        self.lbl_install = QLabel("")
        self.lbl_install.setObjectName("small")
        form.addWidget(self.lbl_install)
        row = QHBoxLayout()
        row.setSpacing(8)
        btn_inst = QPushButton("Add to my app menu")
        btn_inst.clicked.connect(lambda: self._install_menu(True))
        row.addWidget(btn_inst)
        btn_uninst = QPushButton("Remove from app menu")
        btn_uninst.clicked.connect(lambda: self._install_menu(False))
        row.addWidget(btn_uninst)
        row.addStretch(1)
        form.addLayout(row)
        col.addWidget(box)

        # --- advanced (hidden)
        self.btn_adv = QPushButton("Advanced and troubleshooting")
        self.btn_adv.setCheckable(True)
        self.btn_adv.clicked.connect(self._toggle_advanced)
        col.addWidget(self.btn_adv, 0, Qt.AlignmentFlag.AlignLeft)

        self.advanced = QWidget()
        adv = QVBoxLayout(self.advanced)
        adv.setContentsMargins(0, 0, 0, 0)
        adv.setSpacing(16)

        box, form = card("Fine-tuning")
        form.setSpacing(12)
        row = QHBoxLayout()
        row.setSpacing(10)
        row.addWidget(QLabel("Parallel connections"))
        self.sp_workers = QSpinBox()
        self.sp_workers.setRange(1, 16)
        row.addWidget(self.sp_workers)
        row.addSpacing(24)
        row.addWidget(QLabel("Retries per folder"))
        self.sp_retries = QSpinBox()
        self.sp_retries.setRange(1, 10)
        row.addWidget(self.sp_retries)
        row.addStretch(1)
        form.addLayout(row)
        adv.addWidget(box)

        box2, form2 = card("If something goes wrong")
        form2.setSpacing(10)
        row2 = QHBoxLayout()
        row2.setSpacing(10)
        btn_restart = QPushButton("Restart syncing now")
        btn_restart.clicked.connect(self.restart_service)
        row2.addWidget(btn_restart)
        btn_conf = QPushButton("Open settings file")
        btn_conf.clicked.connect(
            lambda: QDesktopServices.openUrl(QUrl.fromLocalFile(str(CONF_FILE))))
        row2.addWidget(btn_conf)
        btn_log = QPushButton("Copy the log command")
        btn_log.clicked.connect(
            lambda: (QApplication.clipboard().setText(
                "journalctl --user -u proton-sync -f"),
                self.sel_hint.setText("copied to clipboard")))
        row2.addWidget(btn_log)
        row2.addStretch(1)
        form2.addLayout(row2)
        adv.addWidget(box2)

        box3, form3 = card("Extra environment variables")
        form3.setSpacing(8)
        self.ed_extra = QPlainTextEdit()
        self.ed_extra.setPlaceholderText("One KEY=VALUE per line, e.g.\nHTTPS_PROXY=http://proxy:3128")
        self.ed_extra.setMinimumHeight(70)
        form3.addWidget(self.ed_extra)
        adv.addWidget(box3)
        adv.addStretch(1)
        self.advanced.setVisible(False)
        col.addWidget(self.advanced)

        # --- save bar
        saver = QHBoxLayout()
        self.btn_save = QPushButton("  Save changes")
        self.btn_save.setObjectName("primary")
        self.btn_save.clicked.connect(self.save_settings)
        saver.addWidget(self.btn_save)
        btn_discard = QPushButton("Discard")
        btn_discard.clicked.connect(self.load_settings)
        saver.addWidget(btn_discard)
        self.save_hint = QLabel("")
        self.save_hint.setObjectName("muted")
        saver.addWidget(self.save_hint)
        saver.addStretch(1)
        col.addLayout(saver)
        col.addStretch(1)

        outer.addWidget(scroll_page(inner), 1)
        return page

    def _toggle_advanced(self, checked: bool):
        self.advanced.setVisible(checked)
        self.btn_adv.setText("Advanced and troubleshooting \u25be" if checked
                             else "Advanced and troubleshooting \u25b8")

    # ---- about --------------------------------------------------------------

    def _about_page(self) -> QWidget:
        page = QWidget()
        outer = QVBoxLayout(page)
        outer.setContentsMargins(30, 30, 30, 20)
        outer.setSpacing(18)
        title = QLabel("About")
        title.setObjectName("pageTitle")
        outer.addWidget(title)

        inner = QWidget()
        col = QVBoxLayout(inner)
        col.setContentsMargins(0, 0, 8, 0)
        col.setSpacing(16)

        box, lay = card()
        hero = QHBoxLayout()
        hero.setSpacing(20)
        big = QLabel()
        big.setPixmap(svg_pixmap(find_icon(), 72))
        hero.addWidget(big, 0, Qt.AlignmentFlag.AlignTop)
        texts = QVBoxLayout()
        texts.setSpacing(3)
        t = QLabel("Proton Drive for Linux")
        t.setObjectName("heroTitle")
        self.about_version = QLabel("Version " + GUI_VERSION)
        self.about_version.setObjectName("muted")
        texts.addWidget(t)
        texts.addWidget(self.about_version)
        texts.addSpacing(8)
        brow = QHBoxLayout()
        btn_repo = QPushButton("Visit the website")
        btn_repo.clicked.connect(lambda: QDesktopServices.openUrl(QUrl(REPO_URL)))
        brow.addWidget(btn_repo)
        brow.addStretch(1)
        texts.addLayout(brow)
        texts.addStretch(1)
        hero.addLayout(texts, 1)
        lay.addLayout(hero)
        col.addWidget(box)

        box, lay = card("What is this?")
        what = QLabel(
            "Keeps a folder on this computer equal to your Proton Drive, "
            "automatically. Built by the community on top of Proton's official "
            "command-line tool. Encryption happens only on your device.")
        what.setWordWrap(True)
        lay.addWidget(what)
        col.addWidget(box)

        box, lay = card("Good to know")
        for text in (
            "Deleted files stay safe unless you enable removal syncing.",
            "Conflicts keep both versions - nothing is lost.",
            "Proton Docs and Sheets stay online only.",
            "Your sign-in stays until you sign out.",
        ):
            line = QLabel("•  " + text)
            line.setWordWrap(True)
            lay.addWidget(line)
        col.addWidget(box)

        box, lay = card("Legal")
        legal = QLabel(
            "Unofficial. Not affiliated with, endorsed, sponsored or supported by "
            "Proton AG, Google LLC or Microsoft Corporation. This app uses the official "
            "Proton Drive CLI (payload MIT, Copyright (c) 2025-2026 Proton AG; source: "
            "github.com/ProtonDriveApps/sdk). All scripts in this repository, including "
            "this app, are MIT-licensed (see LICENSE). 'Proton' and 'Proton Drive' are "
            "trademarks of Proton AG, used here only to identify the product being "
            "packaged. Mentions of other products are feature comparisons only - no code, "
            "assets or configurations of those products are used. Not legal advice.")
        legal.setObjectName("small")
        legal.setWordWrap(True)
        lay.addWidget(legal)
        col.addWidget(box)
        col.addStretch(1)

        outer.addWidget(scroll_page(inner), 1)
        return page

    # ------------------------------------------------------------------ tray

    def _build_tray(self):
        self.tray = None
        if not QSystemTrayIcon.isSystemTrayAvailable():
            return
        self.tray = QSystemTrayIcon(tray_icon(svg_pixmap(find_icon(), 64), "unknown"), self)
        self.tray.setToolTip("Proton Drive")
        menu = QMenu()
        act_show = menu.addAction("Open Proton Drive")
        act_show.triggered.connect(self.show_normal)
        menu.addSeparator()
        self.act_sync = menu.addAction("Sync now")
        self.act_sync.triggered.connect(self.sync_now)
        act_folder = menu.addAction("Open my folder")
        act_folder.triggered.connect(self.open_folder)
        menu.addSeparator()
        self.act_pause = menu.addAction("Pause sync")
        self.act_pause.triggered.connect(self._pause_clicked)
        menu.addSeparator()
        self.act_recent_menu = menu.addMenu("Recent files")
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

    def _nav_fit_items(self):
        """QListWidget's default size hint is a fixed 192px - room for only 4
        themed items. Size the nav to its real content so all entries show."""
        if not self.nav.count():
            return
        row = self.nav.visualItemRect(self.nav.item(0)).height()
        if row > 0:
            self.nav.setMinimumHeight(row * self.nav.count()
                                      + 2 * self.nav.frameWidth() + 4)

    def _switch_page(self, row: int):
        self.stack.setCurrentIndex(max(0, row))
        if row == 1:
            self._cloud_shown()
        if row == 2:
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
        self._home_status = status

        color, title, sub_tpl = STATUS_TEXT.get(status, STATUS_TEXT["unknown"])
        if not self._logged_in:
            self.orb.set_status("unknown")
            self.hero_title.setText("Welcome to Proton Drive")
            self.hero_sub.setText("Sign in to start - your files will appear here.")
        else:
            self.orb.set_status(status)
            self.hero_title.setText(title)
            if status == "synced":
                sub = self._synced_line()
            elif status == "error":
                sub = str(health.get("error") or "See Activity for details.")
            else:
                sub = sub_tpl or "..."
            self.hero_sub.setText(sub[:200])

        self.side_dot.setStyleSheet(f"background: {color}; border-radius: 5px;")
        self.side_status.setText(title)

        self.btn_pause.setText("Pause" if active else "Resume")
        self.btn_pause.setIcon(self.style().standardIcon(
            QStyle.StandardPixmap.SP_MediaPause if active
            else QStyle.StandardPixmap.SP_MediaPlay))
        self.btn_sync.setEnabled(active)
        self.act_sync.setEnabled(active)
        self.act_pause.setText("Pause sync" if active else "Resume sync")

        if self.tray:
            self.tray.setIcon(tray_icon(svg_pixmap(find_icon(), 64), status))
            self.tray.setToolTip(f"Proton Drive - {title}")

        self._poll_notifications(health)
        self._prev_health_status = hstatus

    def _next_poll_text(self) -> str:
        health = read_health()
        try:
            nxt = datetime.datetime.fromisoformat(str(health.get("next_poll")))
            secs = max(0, int((nxt - datetime.datetime.now(nxt.tzinfo)).total_seconds()))
            return f"next in ~{secs // 60}m {secs % 60:02d}s"
        except ValueError:
            return ""

    def _synced_line(self) -> str:
        parts = ["In sync"]
        try:
            last = rel_time(datetime.datetime.fromisoformat(
                str(read_health().get("ts"))))
            if last:
                parts.append(f"checked {last}")
        except ValueError:
            pass
        nxt = self._next_poll_text()
        if nxt:
            parts.append(nxt)
        return "  \u00b7  ".join(parts)

    @guard_slot
    def update_countdown(self):
        if self._logged_in and self._home_status == "synced":
            self.hero_sub.setText(self._synced_line()[:200])

    @guard_slot
    def refresh_logs(self):
        self._log_lines = journal_lines(400)
        self._parse_changes()
        self._rebuild_tray_recent()
        self.render_changes()
        if self.act_stack.currentIndex() == 1:
            self.render_logs()

    def _rebuild_tray_recent(self):
        if not self.tray:
            return
        self.act_recent_menu.clear()
        recent = self._changes[-5:][::-1]
        if not recent:
            empty = self.act_recent_menu.addAction("Nothing yet")
            empty.setEnabled(False)
            return
        for c in recent:
            full = c["path"].rstrip("/")
            name = full.rsplit("/", 1)[-1] or full
            parent = full[:len(full) - len(name)].rstrip("/")
            when = rel_time(c["ts"]) if c["ts"] else ""
            act = self.act_recent_menu.addAction(f"{name[:44]}  \u00b7  {when}")
            act.triggered.connect(lambda _, p=parent: self._open_sync_path(p))

    def _open_sync_path(self, rel: str):
        root = Path(read_conf()["PROTON_SYNC_LOCAL"]).expanduser()
        target = (root / rel) if rel else root
        QDesktopServices.openUrl(QUrl.fromLocalFile(
            str(target if target.is_dir() else root)))

    # ---- activity rendering

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
                label, direction = "Removed", "other"
            else:
                label, direction = "Conflict - both kept", "warn"
            rest = re.sub(r"\s*\(.*\)$", "", rest)
            changes.append({"ts": ts, "label": label, "dir": direction, "path": rest})
        self._changes = changes[-300:]

    @guard_slot
    def render_changes(self, *_):
        if not hasattr(self, "moves_box"):
            return
        # clear old rows
        while self.moves_box.count():
            item = self.moves_box.takeAt(0)
            w = item.widget()
            if w:
                w.deleteLater()

        kind = self.ch_kind.currentIndex() if hasattr(self, "ch_kind") else 0
        needle = self.ch_filter.text().strip().lower() if hasattr(self, "ch_filter") else ""
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
            full = c["path"].rstrip("/")
            name = full.rsplit("/", 1)[-1] or full
            parent = full[:len(full) - len(name)].rstrip("/")
            when = rel_time(c["ts"]) if c["ts"] else ""
            self.moves_box.addWidget(
                make_move_row(c["dir"], name, parent or "/", c["label"], when))
            shown += 1
            if shown >= 120:
                break
        self.moves_box.addStretch(1)
        self.ch_count.setText(f"{shown} items" if shown else "")
        self.moves_empty.setVisible(shown == 0)
        self._render_recent()

    def _render_recent(self):
        while self.recent_box.count():
            item = self.recent_box.takeAt(0)
            w = item.widget()
            if w:
                w.deleteLater()
        for c in self._changes[-6:][::-1]:
            full = c["path"].rstrip("/")
            name = full.rsplit("/", 1)[-1] or full
            when = rel_time(c["ts"]) if c["ts"] else ""
            self.recent_box.addWidget(make_move_row(c["dir"], name, "", c["label"], when))
        self.recent_empty.setVisible(not self._changes)

    def render_logs(self, *_):
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

    @guard_slot
    def refresh_folder_stats(self):
        root = Path(self.ed_local.text().strip()
                    if hasattr(self, "ed_local") and self.ed_local.text().strip()
                    else CONF_DEFAULTS["PROTON_SYNC_LOCAL"]).expanduser()
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
        size = (f"{total / 1_073_741_824:.2f} GB" if total >= 1_073_741_824
                else f"{total / 1_048_576:,.0f} MB")
        self.val_stats.setText(f"{files:,} files  ·  {size}")
        self.folder_name.setText(root.name or "ProtonDrive")

    # ------------------------------------------------------------------ actions

    def sync_now(self):
        if not service_active():
            QMessageBox.information(self, "Sync is paused",
                                    "Syncing is paused. Press Resume first.")
            return
        STATE_DIR.mkdir(parents=True, exist_ok=True)
        WAKE_FILE.touch()
        self.hero_sub.setText("On it - checking for new changes...")

    def open_folder(self):
        conf = read_conf()
        root = Path(conf["PROTON_SYNC_LOCAL"]).expanduser()
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
        systemctl("stop", "proton-sync-autoresume.timer", "proton-sync-autoresume.service")
        if minutes:
            systemd_run(f"--on-active={minutes * 60}", "--unit=proton-sync-autoresume",
                         shutil.which("systemctl") or "systemctl", "--user",
                         "start", JOURNAL_UNIT)
        QTimer.singleShot(700, self.refresh_status)

    def resume_now(self):
        systemctl("stop", "proton-sync-autoresume.timer", "proton-sync-autoresume.service")
        systemctl("start", JOURNAL_UNIT)
        QTimer.singleShot(700, self.refresh_status)

    def restart_service(self):
        systemctl("restart", JOURNAL_UNIT)
        QTimer.singleShot(900, self.refresh_status)

    # ------------------------------------------------------------------ settings

    @guard_slot
    def load_settings(self):
        c = read_conf()
        self.ed_local.setText(c["PROTON_SYNC_LOCAL"])
        self._remote_root = c["PROTON_SYNC_REMOTE"] or "/my-files"
        secs = int(c["PROTON_SYNC_INTERVAL"])
        best = min(range(len(INTERVAL_CHOICES)),
                   key=lambda i: abs(INTERVAL_CHOICES[i][1] - secs))
        self.cmb_interval.setCurrentIndex(best)
        self.chk_inotify.setChecked(c["PROTON_SYNC_INOTIFY"] == "1")
        self.chk_deletes.setChecked(c["PROTON_SYNC_DELETES"] == "1")
        self.sp_workers.setValue(int(c["PROTON_SYNC_WORKERS"]))
        self.sp_retries.setValue(int(c["PROTON_SYNC_RETRIES"]))
        self.ed_exclude.setText(c["PROTON_SYNC_EXCLUDE"])
        self._selection = [s for s in c["PROTON_SYNC_SUBFOLDERS"].split(",") if s]
        self.chk_selective.setChecked(bool(self._selection))
        self.ed_extra.setPlainText("\n".join(read_conf_extras()))
        self.save_hint.setText("")

        theme = self._prefs.value("theme", "system")
        self.cmb_theme.blockSignals(True)
        self.cmb_theme.setCurrentText({"system": "Match my system", "light": "Light",
                                       "dark": "Dark"}.get(theme, "Match my system"))
        self.cmb_theme.blockSignals(False)
        self.chk_starttray.setChecked(self._prefs.value("start_in_tray", False, type=bool))
        self.chk_note_errors.setChecked(self._prefs.value("notify_errors", True, type=bool))
        self.chk_note_conflicts.setChecked(self._prefs.value("notify_conflicts", True, type=bool))
        self.chk_note_done.setChecked(self._prefs.value("notify_complete", False, type=bool))

        exe = shutil.which("systemctl")
        enabled = False
        if exe:
            try:
                enabled = subprocess.run([exe, "--user", "is-enabled", "--quiet",
                                          JOURNAL_UNIT], timeout=10).returncode == 0
            except (subprocess.TimeoutExpired, FileNotFoundError):
                pass
        self.chk_autostart.blockSignals(True)
        self.chk_autostart.setChecked(enabled)
        self.chk_autostart.blockSignals(False)

        self.chk_close_tray.setChecked(
            self._prefs.value("close_to_tray", True, type=bool))
        self.chk_top.blockSignals(True)
        self.chk_top.setChecked(
            self._prefs.value("always_on_top", True, type=bool))
        self.chk_top.blockSignals(False)
        self.chk_updates.setChecked(
            self._prefs.value("updates_auto", True, type=bool))
        self.chk_appstart.blockSignals(True)
        self.chk_appstart.setChecked(autostart_enabled())
        self.chk_appstart.blockSignals(False)
        self.lbl_install.setText(
            "Installed in your app menu."
            if menu_installed() else "Not in your app menu yet.")

        self._sel_load_root()

    def save_settings(self):
        local = self.ed_local.text().strip()
        if not local.startswith("/"):
            QMessageBox.warning(self, "Folder missing",
                                "Please choose a folder to sync first.")
            return
        selection = self._derive_selection() if self.chk_selective.isChecked() else []
        values = {
            "PROTON_SYNC_LOCAL": local,
            "PROTON_SYNC_REMOTE": self._remote_root or "/my-files",
            "PROTON_SYNC_INTERVAL": str(INTERVAL_CHOICES[self.cmb_interval.currentIndex()][1]),
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
            self, "Save changes",
            "Save your settings and restart syncing now?")
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
            self.save_hint.setText("Saved. Your install is from an older version - "
                                   "run scripts/install-sync.sh once to update it.")
        elif restarted:
            self.save_hint.setText("Saved. Syncing restarted.")
        else:
            self.save_hint.setText("Saved, but syncing could not be restarted.")
        QTimer.singleShot(900, self.refresh_status)

    def _theme_changed(self, index: int):
        name = ["system", "light", "dark"][max(0, min(2, index))]
        self._prefs.setValue("theme", name)
        apply_theme(name)
        # themed row heights differ per style - re-fit the nav to its items
        self._nav_fit_items()
        QTimer.singleShot(0, self._nav_fit_items)

    # --- selective sync tree

    def _sel_load_root(self):
        self.sel_tree.clear()
        c = read_conf()
        self._remote_root = c["PROTON_SYNC_REMOTE"] or "/my-files"
        self._selection = [s for s in c["PROTON_SYNC_SUBFOLDERS"].split(",") if s]
        root = QTreeWidgetItem([f"All files in your cloud ({self._remote_root})"])
        root.setData(0, Qt.ItemDataRole.UserRole, "")
        root.setFlags(root.flags() | Qt.ItemFlag.ItemIsUserCheckable
                      | Qt.ItemFlag.ItemIsEnabled)
        root.setCheckState(0, Qt.CheckState.Unchecked if self._selection
                           else Qt.CheckState.Checked)
        self.sel_tree.addTopLevelItem(root)
        placeholder = QTreeWidgetItem(root, ["Loading..."])
        placeholder.setData(0, Qt.ItemDataRole.UserRole, None)
        self.sel_hint.setText("Loading folders...")

        w = Worker(lambda: fetch_remote_folders(self._remote_root, ""))
        w.done.connect(lambda r: self._sel_populate(root, r))
        self._workers.append(w)
        w.start()

    def _sel_populate(self, item, folders):
        if isinstance(folders, Exception):
            folders = []
        self._sel_updating = True
        try:
            item.takeChildren()
            sel = set(self._selection)
            parent_checked = item.checkState(0) == Qt.CheckState.Checked
            for f in folders:
                child = QTreeWidgetItem(item, [f["name"]])
                child.setData(0, Qt.ItemDataRole.UserRole, f["rel"])
                child.setData(0, Qt.ItemDataRole.UserRole + 1, True)
                child.setFlags(child.flags() | Qt.ItemFlag.ItemIsUserCheckable
                               | Qt.ItemFlag.ItemIsEnabled)
                checked = parent_checked or f["rel"] in sel
                child.setCheckState(0, Qt.CheckState.Checked if checked
                                    else Qt.CheckState.Unchecked)
                QTreeWidgetItem(child, ["..."]).setData(0, Qt.ItemDataRole.UserRole, None)
        finally:
            self._sel_updating = False
        if item is self.sel_tree.topLevelItem(0):
            self.sel_hint.setText(f"{len(folders)} folders")

    def _sel_item_changed(self, item):
        """Manual tri-state: check cascades down, parents reflect children."""
        if self._sel_updating:
            return
        self._sel_updating = True
        try:
            state = item.checkState(0)
            if state in (Qt.CheckState.Checked, Qt.CheckState.Unchecked):
                stack = [item]
                while stack:
                    it = stack.pop()
                    for i in range(it.childCount()):
                        c = it.child(i)
                        if c.data(0, Qt.ItemDataRole.UserRole) is None:
                            continue  # placeholder
                        c.setCheckState(0, state)
                        stack.append(c)
            p = item.parent()
            while p is not None:
                states = [p.child(i).checkState(0) for i in range(p.childCount())
                          if p.child(i).data(0, Qt.ItemDataRole.UserRole) is not None]
                if states and all(s == Qt.CheckState.Checked for s in states):
                    p.setCheckState(0, Qt.CheckState.Checked)
                elif any(s != Qt.CheckState.Unchecked for s in states):
                    p.setCheckState(0, Qt.CheckState.PartiallyChecked)
                else:
                    p.setCheckState(0, Qt.CheckState.Unchecked)
                p = p.parent()
        finally:
            self._sel_updating = False

    def _sel_expand(self, item):
        if item.data(0, Qt.ItemDataRole.UserRole + 1):
            return
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

    def _autostart_toggled(self, checked: bool):
        exe = shutil.which("systemctl")
        if not exe:
            return
        ok = subprocess.run([exe, "--user", "enable" if checked else "disable",
                             JOURNAL_UNIT],
                            capture_output=True, timeout=15).returncode == 0
        self.save_hint.setText("Will start when you log in." if ok
                               else "Could not change start-up.")

    def _apply_window_prefs(self):
        """Floating window: remembered size + stay-on-top flag.

        Remembered sizes are floored to the no-scroll layout (920x780) so a
        window saved before this version never comes back with content cut off.
        """
        if self._prefs.value("always_on_top", True, type=bool):
            self.setWindowFlag(Qt.WindowType.WindowStaysOnTopHint, True)
        w, h = self.width(), self.height()
        try:
            sw, sh = (int(v)
                      for v in str(self._prefs.value("window_size", "")).split("x"))
            if sw >= 700 and sh >= 480:
                w, h = sw, sh
        except (ValueError, TypeError):
            pass
        w = max(w, 920)
        h = max(h, 780)
        geo = self.screen().availableGeometry() if self.screen() else None
        if geo:
            self.resize(min(w, geo.width()), min(h, geo.height()))
        else:
            self.resize(w, h)

    def _remember_window_size(self):
        self._prefs.setValue("window_size", f"{self.width()}x{self.height()}")

    def _always_on_top_toggled(self, checked: bool):
        self._prefs.setValue("always_on_top", checked)
        flags = self.windowFlags()
        if checked:
            flags |= Qt.WindowType.WindowStaysOnTopHint
        else:
            flags &= ~Qt.WindowType.WindowStaysOnTopHint
        self.setWindowFlags(flags)   # Qt re-creates the window (it hides)
        if not self._quitting:
            self.show()

    def _app_autostart_toggled(self, checked: bool):
        set_autostart(checked)

    def _install_menu(self, on: bool):
        set_menu_installed(on)
        self.lbl_install.setText(
            "Installed in your app menu." if on
            else "Not in your app menu yet.")

    # --- updates

    def _check_updates_clicked(self):
        self.lbl_update.setText("Checking for updates...")
        w = Worker(lambda: (check_for_release(), self._identity.get("version", "")))
        w.done.connect(self._update_checked)
        self._workers.append(w)
        w.start()

    def _update_checked(self, result):
        if isinstance(result, Exception):
            self.lbl_update.setText("Could not check for updates right now.")
            return
        rel, cli_version = result
        self._release = rel
        m = re.search(r"@(\d+\.\d+\.\d+)", cli_version or "")
        cur = m.group(1) if m else ""

        def newer(a: str, b: str) -> bool:
            try:
                return (tuple(int(x) for x in a.split("."))
                        > tuple(int(x) for x in b.split(".")))
            except ValueError:
                return False

        have_update = bool(
            cur and rel.get("gui")
            and (newer(rel["cli"], cur)
                 or (rel["cli"] == cur and newer(rel["gui"], GUI_VERSION))))
        if have_update:
            self.lbl_update.setText(
                f"Update available: CLI {rel['cli']} + GUI {rel['gui']} "
                f"(you have {cur} + {GUI_VERSION}).")
            self.btn_dl_update.setVisible(bool(os.environ.get("APPIMAGE")))
            if self.tray and self._cool_ok("update", 6 * 3600):
                self._notify("Update available",
                             f"Proton Drive {rel['tag']} is ready to install.")
        else:
            self.lbl_update.setText(
                f"You're up to date (latest release: {rel['tag']}, "
                f"{rel['published']}).")
            self.btn_dl_update.setVisible(False)

    def _download_update(self):
        if not self._release:
            return
        rel = self._release
        arch = os.uname().machine
        self.btn_dl_update.setEnabled(False)
        self.lbl_update.setText("Downloading and verifying the update...")
        w = Worker(lambda: install_update_and_restart(download_update(rel, arch)))
        w.done.connect(self._update_downloaded)
        self._workers.append(w)
        w.start()

    def _update_downloaded(self, result):
        self.btn_dl_update.setEnabled(True)
        if isinstance(result, Exception):
            self.lbl_update.setText(f"Update failed: {result}")
            return
        self.lbl_update.setText("Updated - the new version is starting...")
        QTimer.singleShot(1500, self.quit_app)

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
        self._logged_in = bool(logged_in)
        self.avatar.set_email(email)
        self.val_email.setText(email or "You're not signed in yet")
        self.badge_login.setText("Signed in" if logged_in else "Not signed in")
        self.btn_login.setVisible(not logged_in)
        self.btn_logout.setVisible(logged_in)
        ver = re.sub(r"\+\w+$", "", str(result.get("version", "?")))
        self.about_version.setText(f"Version {GUI_VERSION}  ·  Engine {ver}")
        self.about_version.setToolTip(str(result.get("version", "")))
        self.refresh_status()

    def login_terminal(self):
        cli = resolve_cli()
        if not cli:
            QMessageBox.warning(self, "Not found",
                                "The Proton command-line tool is missing. "
                                "Reinstall the app and try again.")
            return
        for name, args in (("konsole", ["-e"]), ("kitty", []), ("alacritty", ["-e"]),
                           ("xfce4-terminal", ["-e"]), ("gnome-terminal", ["--"]),
                           ("xterm", ["-e"])):
            exe = shutil.which(name)
            if exe:
                script = (f'{shlex.quote(cli)} auth login; echo; '
                          'echo "You can close this window now."')
                subprocess.Popen([exe, *args, "sh", "-c", script])
                QTimer.singleShot(10000, self.load_identity)
                return
        QMessageBox.information(
            self, "Sign in",
            "A terminal window could not be opened automatically.\n\n"
            "Please open a terminal yourself and run:\n\n"
            "    proton-drive auth login")

    def logout(self):
        if QMessageBox.question(
                self, "Sign out",
                "Sign out of Proton on this computer?\n\n"
                "Your synced files stay on this computer.") \
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
        if not desktop_notification(title, msg) and self.tray:
            self.tray.showMessage(title, msg, QSystemTrayIcon.MessageIcon.Information, 4500)

    def _poll_notifications(self, health: dict):
        lines = journal_lines(60)
        for ln in lines:
            ts = journal_ts(ln)
            if ts is None or ts <= self._last_seen_ts:
                continue
            self._last_seen_ts = ts
            body = journal_body(ln)
            m = CHANGE_RE.search(body)
            name = (m.group(2).strip().split("/")[-1] if m else "")[:60]
            if "CONFLICT" in body and self.chk_note_conflicts.isChecked() \
                    and self._cool_ok("conflict"):
                self._notify("Two versions were kept",
                             f"'{name}' changed in two places - both copies are safe.")
            elif ("ERROR" in body or "FAILED" in body) \
                    and self.chk_note_errors.isChecked() and self._cool_ok("error"):
                self._notify("Sync problem", body[:150])
        if (self._prev_health_status == "syncing" and health.get("status") == "ok"
                and self.chk_note_done.isChecked()
                and (health.get("uploaded", 0) or health.get("downloaded", 0))
                and self._cool_ok("done", 60)):
            self._notify("All done",
                         f"{health.get('uploaded', 0)} file(s) uploaded, "
                         f"{health.get('downloaded', 0)} downloaded.")

    # ------------------------------------------------------------------ window

    def quit_app(self):
        self._quitting = True
        self._remember_window_size()
        if self.tray:
            self.tray.hide()
        QApplication.quit()

    def closeEvent(self, event: QCloseEvent):  # noqa: N802 - Qt naming
        self._remember_window_size()
        close_to_tray = self._prefs.value("close_to_tray", True, type=bool)
        if (not self._quitting and close_to_tray and self.tray
                and QSystemTrayIcon.isSystemTrayAvailable()):
            event.ignore()
            self.hide()
            if not self._tray_warned:
                self._tray_warned = True
                if not desktop_notification(
                        "Proton Drive",
                        "Still running - syncing continues in the background. "
                        "Right-click the tray icon to quit."):
                    self.tray.showMessage(
                        "Proton Drive",
                        "Still running - syncing continues in the "
                        "background. Right-click the tray icon to quit.",
                        QSystemTrayIcon.MessageIcon.Information, 4000)
            return
        event.accept()


def scroll_page(inner: QWidget) -> QScrollArea:
    sc = QScrollArea()
    sc.setWidgetResizable(True)
    sc.setWidget(inner)
    sc.setFrameShape(QFrame.Shape.NoFrame)
    sc.viewport().setAutoFillBackground(False)
    return sc


# --------------------------------------------------------------------------- main


def main() -> int:
    smoke = "--smoke" in sys.argv
    if smoke:
        os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

    # Wayland compositors ignore always-on-top requests (no protocol for
    # it), so when the floating-on-top preference is on, run the window on
    # X11/XWayland where the flag works. No-op on plain X11 sessions.
    prefs = QSettings("ProtonDriveCLI", "ProtonDriveGUI")
    if (prefs.value("always_on_top", True, type=bool)
            and os.environ.get("XDG_SESSION_TYPE") == "wayland"
            and not smoke):
        os.environ["QT_QPA_PLATFORM"] = "xcb"

    app = QApplication(sys.argv)
    app.setApplicationName("Proton Drive")
    app.setOrganizationName("ProtonDriveCLI")
    app.setStyle("Fusion")
    font = QFont("Inter")
    font.setPointSize(10)
    app.setFont(font)

    theme = prefs.value("theme", "system")
    apply_theme(theme if theme in ("system", "light", "dark") else "system")

    STATE_DIR.mkdir(parents=True, exist_ok=True)
    lock = QLockFile(str(LOCK_FILE))
    if not smoke and not lock.tryLock(100):
        QMessageBox.information(None, "Already running",
                                "Proton Drive is already open.")
        return 0

    win = MainWindow()
    win.chk_autostart.toggled.connect(win._autostart_toggled)
    win.chk_starttray.toggled.connect(
        lambda v: win._prefs.setValue("start_in_tray", v))
    win.chk_close_tray.toggled.connect(
        lambda v: win._prefs.setValue("close_to_tray", v))
    win.chk_top.toggled.connect(win._always_on_top_toggled)
    win.chk_updates.toggled.connect(
        lambda v: win._prefs.setValue("updates_auto", v))
    win.chk_appstart.toggled.connect(win._app_autostart_toggled)
    for chk, key in ((win.chk_note_errors, "notify_errors"),
                     (win.chk_note_conflicts, "notify_conflicts"),
                     (win.chk_note_done, "notify_complete")):
        chk.toggled.connect(lambda v, k=key: win._prefs.setValue(k, v))

    if smoke or not prefs.value("start_in_tray", False, type=bool):
        win.show()
        if app.platformName() == "xcb":
            geo = app.primaryScreen().availableGeometry()
            win.move(geo.center() - win.rect().center())
    else:
        if win.tray:
            desktop_notification(
                "Proton Drive",
                "Running quietly in the tray. Click the icon to open.")

    if prefs.value("updates_auto", True, type=bool):
        QTimer.singleShot(5000, win._check_updates_clicked)

    if smoke:
        QTimer.singleShot(2500, app.quit)
    rc = app.exec()
    lock.unlock()
    return rc


if __name__ == "__main__":
    sys.exit(main())
