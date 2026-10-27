# Fathom. From the root: make fathom <target>. `make fathom` lists the targets.
#
# fathom-desktop (ui/src-tauri) is left out: it needs a built ui/dist. The root's
# `make build-linux` builds and packages it.

PROJECT        := fathom
CRATES         := -p fathom -p fathom-daemon -p pipeline -p fathom-server -p fathom-devbridge
WEB_PKG        := fathom-ui
BINDINGS_CRATE := fathom
BINDINGS_DIR   := ui/src/protocol/generated

include ../mk/project.mk
