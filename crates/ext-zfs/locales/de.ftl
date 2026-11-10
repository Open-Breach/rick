# ext-zfs — Deutsch.

zfs-name-empty = einen Namen eingeben
zfs-name-character = „{ $character }“ ist in einem Snapshot-Namen nicht erlaubt
zfs-name-too-long = der Name ist zu lang

zfs-versions-of = Versionen von { $name }

zfs-zfs = ZFS
zfs-browse-snapshots = Snapshots durchsuchen
zfs-back-to-live-folder = Zurück zum aktuellen Ordner
zfs-versions = Versionen
zfs-no-snapshot-holds-folder = kein Snapshot enthält diesen Ordner
zfs-select-one-file = zuerst eine Datei auswählen
zfs-restore = Wiederherstellen
zfs-select-items-to-restore = zuerst auswählen, was wiederhergestellt werden soll
zfs-restore-replaces = Diese aktuellen Einträge werden in den Papierkorb verschoben und durch ihre Snapshot-Version ersetzt:
    { $items }
zfs-restoring = { $count ->
    [one] Einen Eintrag
   *[other] { $count } Einträge
} aus einem Snapshot wiederherstellen
zfs-not-restored = Nicht wiederhergestellt, der aktuelle Eintrag ließ sich nicht in den Papierkorb verschieben oder kopieren:
    { $items }
zfs-create-snapshot = Snapshot erstellen
zfs-create-snapshot-of = Snapshot von { $dataset } erstellen
zfs-create = Erstellen
zfs-snapshot-name = Name des Snapshots
zfs-include-children = Untergeordnete Datasets einschließen
zfs-creating = Snapshot { $name } erstellen
zfs-snapshot-exists = Ein Snapshot namens { $name } existiert bereits; nichts wurde geändert.
zfs-rename-snapshot = Snapshot umbenennen
zfs-rename = Umbenennen
zfs-renaming = Snapshot { $name } umbenennen
zfs-select-one-snapshot = zuerst einen Snapshot auswählen
zfs-destroy-snapshot = Snapshot löschen
zfs-select-snapshots = zuerst Snapshots auswählen
zfs-destroy-confirm = Diese Snapshots löschen? Das lässt sich nicht rückgängig machen.
    { $names }
zfs-destroying = { $count ->
    [one] Einen Snapshot
   *[other] { $count } Snapshots
} löschen
zfs-snapshot-busy = { $name } ist gesperrt (hold) oder hat abhängige Klone und bleibt erhalten: { $reason }
zfs-run-as-administrator = Als Administrator ausführen?
zfs-run-as-administrator-ok = Als Administrator ausführen
zfs-permission-denied-ask = ZFS hat „{ $name }“ mangels Berechtigung abgelehnt. Noch einmal als Administrator ausführen?
zfs-as-administrator = { $name } (als Administrator)
zfs-permission-denied = Zugriff verweigert; nichts wurde geändert.
zfs-needs-permission = Dir fehlen dafür die Rechte auf { $dataset }, und keine Administrator-Abfrage ist verfügbar. Ein Administrator kann sie so vergeben:
    { $command }
