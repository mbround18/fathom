# Depths (spec 050). From the root: make depths <target>. `make depths` lists the targets.

PROJECT        := depths
# The same crates scripts/ci.sh's Depths job tests, including the shared ones Depths owns.
CRATES         := -p depths-core -p depths-protocol -p depths-runtime-cli -p depths-stacks-fs \
                  -p depths-volumes -p depths-server -p depths-volume-helper \
                  -p container-runtime -p web-auth -p audit-log
WEB_PKG        := @companion/depths-web
BINDINGS_CRATE := depths-protocol
BINDINGS_DIR   := apps/depths/web/src/protocol/generated

include ../../mk/project.mk
