#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# A small Qt Widgets window with the controls a real QWidget app is made of, so
# the palette in sdk/theme/src/qt.rs can be looked at beside an Arlen window:
# a menu bar and a toolbar, a list with a selected row, tabs, buttons (one of
# them default, one disabled), a line edit with a placeholder, a check box, a
# combo, a slider, a progress bar and a status bar. Fusion draws everything
# from the QPalette qt6ct (or qt5ct) hands it, which is what gets photographed.
#
# `QT_GALLERY_MAJOR=5` runs the same window on PyQt5, so the one colour scheme
# the theme writes for both can be looked at under both.
import importlib
import os
import sys

QT = "PyQt5" if os.environ.get("QT_GALLERY_MAJOR") == "5" else "PyQt6"
QtWidgets = importlib.import_module(f"{QT}.QtWidgets")
QtCore = importlib.import_module(f"{QT}.QtCore")
globals().update({name: getattr(QtWidgets, name) for name in (
    "QApplication",
    "QCheckBox",
    "QComboBox",
    "QHBoxLayout",
    "QLabel",
    "QLineEdit",
    "QListWidget",
    "QMainWindow",
    "QProgressBar",
    "QPushButton",
    "QSlider",
    "QTabWidget",
    "QToolBar",
    "QVBoxLayout",
    "QWidget",
)})
Qt = QtCore.Qt

app = QApplication(sys.argv)
win = QMainWindow()
win.setWindowTitle(f"{QT[2:]} gallery")
win.resize(560, 520)

menu = win.menuBar()
for name in ("File", "Edit", "View", "Help"):
    menu.addMenu(name)
bar = QToolBar()
bar.addAction("Open")
bar.addAction("Save")
bar.addSeparator()
bar.addAction("Print")
win.addToolBar(bar)

root = QWidget()
outer = QHBoxLayout(root)

folders = QListWidget()
folders.addItems(["Inbox", "Archive", "Sent", "Trash"])
folders.setCurrentRow(0)
folders.setMaximumWidth(150)
outer.addWidget(folders)

tabs = QTabWidget()
page = QWidget()
col = QVBoxLayout(page)
col.addWidget(QLabel("Name"))
edit = QLineEdit()
edit.setPlaceholderText("Type a name")
col.addWidget(edit)
combo = QComboBox()
combo.addItems(["Date", "Sender", "Subject"])
col.addWidget(combo)
col.addWidget(QCheckBox("Show unread only", checked=True))
disabled = QCheckBox("Sync (no account)")
disabled.setEnabled(False)
col.addWidget(disabled)
slider = QSlider(Qt.Orientation.Horizontal if QT == "PyQt6" else Qt.Horizontal)
slider.setValue(60)
col.addWidget(slider)
progress = QProgressBar()
progress.setValue(60)
col.addWidget(progress)
buttons = QHBoxLayout()
buttons.addWidget(QPushButton("Cancel"))
save = QPushButton("Save")
save.setDefault(True)
buttons.addWidget(save)
later = QPushButton("Later")
later.setEnabled(False)
buttons.addWidget(later)
col.addLayout(buttons)
col.addStretch()
tabs.addTab(page, "General")
tabs.addTab(QWidget(), "Advanced")
outer.addWidget(tabs)

win.setCentralWidget(root)
win.statusBar().showMessage("Reading your mail.")
win.show()
sys.exit(app.exec() if QT == "PyQt6" else app.exec_())
