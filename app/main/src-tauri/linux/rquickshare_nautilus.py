# Adds "Send with Quick Share" to the Nautilus context menu.
# Installed by RQuickShare (Settings > File manager integration).
# Needs nautilus-python (python-nautilus / nautilus-python / python3-nautilus).

import subprocess

from gi.repository import GObject, Nautilus

EXEC = "@EXEC@"


class RQuickShareMenuProvider(GObject.GObject, Nautilus.MenuProvider):
    # Nautilus 43+ calls get_file_items(files), older versions (window, files).
    def get_file_items(self, *args):
        files = args[-1]
        if not files or any(f.get_uri_scheme() != "file" for f in files):
            return []

        item = Nautilus.MenuItem(
            name="RQuickShare::send",
            label="Send with Quick Share",
            icon="rquickshare",
        )
        item.connect("activate", self._send, files)
        return [item]

    def _send(self, _item, files):
        paths = [f.get_location().get_path() for f in files]
        subprocess.Popen([EXEC, "--send", *paths], start_new_session=True)
