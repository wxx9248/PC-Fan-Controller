# Build and install fan-controllerd for one installation target.
#
# Everything that differs between a hand-built install and a distribution
# package lives in config/targets/<target>.conf. scripts/render-config
# substitutes those values into config/templates/*.in, and this file asks that
# same script where each rendered file belongs, so the paths are stated once.
# Adding a distribution means adding one file under config/targets/.
#
#   make                             build and render, as yourself
#   sudo make install                copy the result into place (see make show)
#   make deb                         build the Debian package in targets/debian
#   make targets                     list the installation targets
#   make show                        print the resolved layout
#
# A machine's own values are given on the command line rather than written into
# a committed file, and they apply when rendering:
#
#   make CONTROLLER_SERIAL=AA:BB:CC:DD:EE:FF DISKS='["ata-MODEL_SERIAL"]'
#
# Nothing here builds as root. Building as root leaves root-owned files in
# target/ and build/ that break the next ordinary build, so the build steps
# refuse it, and install only copies what an unprivileged build produced.
#
# Per-target packaging lives in targets/<target>/. Adding a distribution means a
# value set in config/targets/ and, if it needs packaging metadata, a directory
# beside this one.

TARGET ?= local
DESTDIR ?=
CARGO ?= cargo
CARGO_FLAGS ?= --locked --release

RENDER := ./scripts/render-config
BUILDDIR := build/$(TARGET)
TEMPLATES := $(wildcard config/templates/*.in)
RENDERED := $(patsubst config/templates/%.in,$(BUILDDIR)/%,$(TEMPLATES))
RENDER_INPUTS := $(TEMPLATES) config/targets/common.conf config/targets/$(TARGET).conf $(RENDER)
BINARY := $(or $(CARGO_TARGET_DIR),target)/release/fan-controllerd
SOURCES := $(shell find src -name '*.rs') Cargo.toml Cargo.lock

ifeq ($(filter $(TARGET),$(shell $(RENDER) --list-targets)),)
  $(error unknown TARGET '$(TARGET)'; available: $(shell $(RENDER) --list-targets | tr '\n' ' '))
endif

# Ask the renderer rather than restating any path here. The layout is per
# target and cannot be overridden, so these always match the rendered files:
# a BINDIR given here would move the binary away from the unit's ExecStart.
LAYOUT := BINDIR CONFIG_FILE UNITDIR UDEVRULESDIR MODULESLOADDIR MANDIR
$(foreach v,$(LAYOUT),$(if $(filter command line,$(origin $(v))),$(error $(v) belongs to the '$(TARGET)' target's layout and cannot be overridden; stage with DESTDIR, or add a target under config/targets/)))
ask = $(shell $(RENDER) --target $(TARGET) --print $(1))
BINDIR := $(call ask,BINDIR)
CONFIG_FILE := $(call ask,CONFIG_FILE)
UNITDIR := $(call ask,UNITDIR)
UDEVRULESDIR := $(call ask,UDEVRULESDIR)
MODULESLOADDIR := $(call ask,MODULESLOADDIR)
MANDIR := $(call ask,MANDIR)

# Values from config/targets/ given on this make command line.
OVERRIDDEN := $(foreach v,$(shell $(RENDER) --list-settable),$(if $(filter command line,$(origin $(v))),$(v)))
$(foreach v,$(OVERRIDDEN),$(if $(findstring ',$($(v))),$(error $(v) contains a single quote, which this Makefile cannot pass on)))
OVERRIDES := $(foreach v,$(OVERRIDDEN),--set '$(v)=$($(v))')

refuse_root = @if [ "$$(id -u)" = 0 ]; then \
		echo "make $@: refusing to build as root. Run 'make' as yourself, then 'sudo make install'." >&2; \
		exit 1; \
	fi

# An install given values on its command line must find them in what was
# rendered, since install itself never renders.
check_overrides = @$(foreach v,$(OVERRIDDEN),grep -qxF -- '$(v)=$($(v))' $(BUILDDIR)/.overrides 2>/dev/null || { \
		echo "make $@: $(BUILDDIR) was not rendered with $(v)=$($(v)). Run 'make $(v)=...' as yourself first; install only copies." >&2; \
		exit 1; };) true

.PHONY: all build test render install install-udev deb clean distclean targets show
.DELETE_ON_ERROR:

all: build render

build:
	$(refuse_root)
	$(CARGO) build $(CARGO_FLAGS)
	@# cargo leaves an unchanged binary's timestamp alone, and install compares
	@# that timestamp with the sources.
	@touch $(BINARY)

test:
	$(refuse_root)
	$(CARGO) test $(CARGO_FLAGS)

# Cheap enough to redo every time, which keeps it honest about edits to a
# template or a value set.
render:
	$(refuse_root)
	@$(RENDER) --target $(TARGET) $(OVERRIDES) --outdir $(BUILDDIR) >/dev/null
	@echo "rendered $(TEMPLATES) for target '$(TARGET)' into $(BUILDDIR)"

# What install copies. These rules never build anything: run as root, they would
# leave root-owned files behind, so a missing or stale input is an error instead.
$(BINARY): $(SOURCES)
	@echo "$@ is missing or older than its sources. Run 'make' as yourself first." >&2; exit 1

$(RENDERED): $(RENDER_INPUTS)
	@echo "$@ is missing or older than its templates. Run 'make' as yourself first." >&2; exit 1

install: $(BINARY) $(RENDERED)
	$(check_overrides)
	install -D -m 0755 $(BINARY) $(DESTDIR)$(BINDIR)/fan-controllerd
	install -D -m 0644 $(BUILDDIR)/fan-controllerd.service $(DESTDIR)$(UNITDIR)/fan-controllerd.service
	install -D -m 0644 $(BUILDDIR)/fan-controllerd.conf $(DESTDIR)$(MODULESLOADDIR)/fan-controllerd.conf
	install -D -m 0644 doc/fan-controllerd.8 $(DESTDIR)$(MANDIR)/man8/fan-controllerd.8
# Never clobber a configuration someone has already edited. Harmless for a
# package build, where DESTDIR is a staging tree that cannot contain one yet.
	@if [ -e "$(DESTDIR)$(CONFIG_FILE)" ]; then \
		echo "keeping existing $(DESTDIR)$(CONFIG_FILE); rendered copy is $(BUILDDIR)/fan-controllerd.toml"; \
	else \
		install -D -m 0644 $(BUILDDIR)/fan-controllerd.toml $(DESTDIR)$(CONFIG_FILE); \
	fi

# Optional: the /dev/<symlink> convenience rule. The by-id path works without it,
# so it is not part of a default install.
install-udev: $(BUILDDIR)/99-fan-controller.rules
	$(check_overrides)
	install -D -m 0644 $(BUILDDIR)/99-fan-controller.rules \
		$(DESTDIR)$(UDEVRULESDIR)/99-fan-controller.rules

# dpkg insists the packaging directory be literally ./debian at the root of the
# source tree, so it is staged as a symlink for the duration of the build and
# removed afterwards whether the build succeeded or not. Keeping the real files
# under targets/debian/ is what lets a second distribution sit beside it.
deb:
	$(refuse_root)
	@test -f targets/debian/rules || { echo "no targets/debian/rules to package with" >&2; exit 1; }
	@if [ -e debian ] && [ ! -L debian ]; then \
		echo "./debian exists and is not the staged symlink; refusing to replace it" >&2; \
		exit 1; \
	fi
	ln -sfn targets/debian debian; \
	rc=0; dpkg-buildpackage -us -uc -b || rc=$$?; \
	rm -f debian; \
	exit $$rc

# Deliberately does NOT remove the staged ./debian symlink. dpkg-buildpackage
# runs `debian/rules clean` as its first step, which lands here, and pulling the
# symlink out from under the build leaves dh unable to find debian/control.
clean:
	$(CARGO) clean
	rm -rf build

# For a human who wants the tree pristine, including a symlink left behind by an
# interrupted `make deb`.
distclean: clean
	rm -f debian

targets:
	@$(RENDER) --list-targets

show:
	@echo "TARGET         $(TARGET)"
	@echo "BINDIR         $(BINDIR)"
	@echo "CONFIG_FILE    $(CONFIG_FILE)"
	@echo "UNITDIR        $(UNITDIR)"
	@echo "UDEVRULESDIR   $(UDEVRULESDIR)"
	@echo "MODULESLOADDIR $(MODULESLOADDIR)"
	@echo "MANDIR         $(MANDIR)"
