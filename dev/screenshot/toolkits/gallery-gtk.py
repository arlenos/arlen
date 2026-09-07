#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# A small libadwaita window with the controls a real GTK4 app is made of, so the
# colour mapping in sdk/theme/src/gtk.rs can be looked at beside an Arlen
# window: header bar with a title and a menu, a sidebar next to a content view,
# the three button kinds, entry and switch rows, a check and a combo, a list
# with a selected row, a level bar, disabled controls, and a toast. Nothing
# here is styled by hand; every colour comes from the named colours the theme
# writes into gtk-4.0/gtk.css, which is the whole point of photographing it.
import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, Gtk  # noqa: E402


class Gallery(Adw.Application):
    def __init__(self):
        super().__init__(application_id="dev.arlen.toolkit-gallery-gtk")

    def do_activate(self):
        win = Adw.ApplicationWindow(application=self, title="GTK4 gallery")
        win.set_default_size(560, 520)

        split = Adw.NavigationSplitView()
        split.set_min_sidebar_width(150)
        split.set_max_sidebar_width(180)

        side_list = Gtk.ListBox(css_classes=["navigation-sidebar"])
        for label in ("Inbox", "Archive", "Sent", "Trash"):
            row = Gtk.ListBoxRow()
            row.set_child(Gtk.Label(label=label, xalign=0, margin_top=6, margin_bottom=6))
            side_list.append(row)
        side_list.select_row(side_list.get_row_at_index(0))
        side_page = Adw.NavigationPage(title="Folders")
        side_tb = Adw.ToolbarView()
        side_tb.add_top_bar(Adw.HeaderBar())
        side_tb.set_content(side_list)
        side_page.set_child(side_tb)
        split.set_sidebar(side_page)

        content = Adw.ToolbarView()
        header = Adw.HeaderBar()
        menu = Gtk.MenuButton(icon_name="open-menu-symbolic")
        header.pack_end(menu)
        content.add_top_bar(header)

        page = Adw.PreferencesPage()
        group = Adw.PreferencesGroup(title="Controls", description="Every kind a settings page uses.")
        entry = Adw.EntryRow(title="Name")
        entry.set_text("Rosa Winter")
        group.add(entry)
        switch = Adw.SwitchRow(title="Reminders", subtitle="Notify before an event", active=True)
        group.add(switch)
        combo = Adw.ComboRow(title="Sort by")
        combo.set_model(Gtk.StringList.new(["Date", "Sender", "Subject"]))
        group.add(combo)
        check_row = Adw.ActionRow(title="Show unread only")
        check = Gtk.CheckButton(active=True, valign=Gtk.Align.CENTER)
        check_row.add_suffix(check)
        group.add(check_row)
        disabled = Adw.SwitchRow(title="Sync", subtitle="No account", sensitive=False)
        group.add(disabled)
        page.add(group)

        buttons = Adw.PreferencesGroup(title="Buttons")
        box = Gtk.Box(spacing=8, margin_top=6, margin_bottom=6)
        box.append(Gtk.Button(label="Cancel"))
        box.append(Gtk.Button(label="Save", css_classes=["suggested-action"]))
        box.append(Gtk.Button(label="Delete", css_classes=["destructive-action"]))
        box.append(Gtk.Button(label="Later", sensitive=False))
        buttons.add(box)
        level = Gtk.LevelBar(value=0.6, margin_top=4, margin_bottom=8)
        buttons.add(level)
        page.add(buttons)

        toast = Adw.ToastOverlay()
        toast.set_child(page)
        content.set_content(toast)
        content_page = Adw.NavigationPage(title="Settings", child=content)
        split.set_content(content_page)

        win.set_content(split)
        win.present()
        toast.add_toast(Adw.Toast(title="Saved to Drafts", timeout=0))


Gallery().run()
