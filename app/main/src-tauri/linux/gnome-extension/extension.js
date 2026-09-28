// A Quick Settings tile for QuickShare, driven over its D-Bus service. The
// tile shows while the app runs: on means visible to everyone nearby.
import Gio from "gi://Gio"
import GObject from "gi://GObject"

import { Extension } from "resource:///org/gnome/shell/extensions/extension.js"
import * as Main from "resource:///org/gnome/shell/ui/main.js"
import * as PopupMenu from "resource:///org/gnome/shell/ui/popupMenu.js"
import {
  QuickMenuToggle,
  SystemIndicator,
} from "resource:///org/gnome/shell/ui/quickSettings.js"

const BUS_NAME = "dev.mandre.RQuickShare"
const OBJECT_PATH = "/dev/mandre/RQuickShare"
const RQuickShareProxy = Gio.DBusProxy.makeProxyWrapper(`
<node>
  <interface name="dev.mandre.RQuickShare1">
    <method name="Show"/>
    <property name="Visibility" type="s" access="readwrite"/>
    <property name="DeviceName" type="s" access="read"/>
  </interface>
</node>`)

const SUBTITLES = {
  visible: "Everyone",
  temporary: "Everyone for a minute",
  hidden: "Hidden",
}

const QuickShareToggle = GObject.registerClass(
  class QuickShareToggle extends QuickMenuToggle {
    _init(icon) {
      super._init({ title: "Quick Share", gicon: icon, toggleMode: true })
      this.menu.setHeader(icon, "Quick Share")

      this._forMinute = this.menu.addAction("Visible for a minute", () =>
        this._setVisibility("temporary")
      )
      this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem())
      this.menu.addAction("Open QuickShare", () => {
        this._proxy?.ShowAsync().catch(logError)
        Main.panel.closeQuickSettings()
      })

      this.connect("clicked", () =>
        this._setVisibility(this.checked ? "visible" : "hidden")
      )
    }

    setProxy(proxy) {
      this._proxy = proxy
      this.visible = proxy !== null
      if (!proxy) return
      proxy.connect("g-properties-changed", () => this._sync())
      this._sync()
    }

    _setVisibility(visibility) {
      if (this._proxy) this._proxy.Visibility = visibility
    }

    _sync() {
      const visibility = this._proxy.Visibility ?? "hidden"
      this.checked = visibility !== "hidden"
      this.subtitle = SUBTITLES[visibility] ?? null
      this.menu.setHeader(this.gicon, "Quick Share", this._proxy.DeviceName)
    }
  }
)

export default class RQuickShareExtension extends Extension {
  enable() {
    const icon = Gio.icon_new_for_string(`${this.path}/rquickshare-symbolic.svg`)
    this._toggle = new QuickShareToggle(icon)
    this._toggle.visible = false
    this._indicator = new SystemIndicator()
    this._indicator.quickSettingsItems.push(this._toggle)
    Main.panel.statusArea.quickSettings.addExternalIndicator(this._indicator)

    this._watch = Gio.bus_watch_name(
      Gio.BusType.SESSION,
      BUS_NAME,
      Gio.BusNameWatcherFlags.NONE,
      () => this._connect(),
      () => this._toggle?.setProxy(null)
    )
  }

  _connect() {
    new RQuickShareProxy(
      Gio.DBus.session,
      BUS_NAME,
      OBJECT_PATH,
      (proxy, error) => {
        if (error) return logError(error)
        this._toggle?.setProxy(proxy)
      }
    )
  }

  disable() {
    Gio.bus_unwatch_name(this._watch)
    this._indicator.quickSettingsItems.forEach((item) => item.destroy())
    this._indicator.destroy()
    this._indicator = null
    this._toggle = null
  }
}
