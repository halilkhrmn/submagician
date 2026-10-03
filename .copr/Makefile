# Fedora COPR, source type "SCM" with the "make_srpm" method: COPR clones the repository and
# runs `make -f .copr/Makefile srpm outdir=<dir>` as root in a Fedora chroot with network.
# See packaging/fedora/make-srpm.sh (it builds the newest release tag, not main). The release
# workflow sends its own SRPM (with the provider keys) instead; this is the fallback.
outdir ?= $(CURDIR)/target/srpm
top := $(abspath $(dir $(lastword $(MAKEFILE_LIST)))..)

srpm:
	dnf -y install cargo git rpm-build xz
	git config --global --add safe.directory '*'
	sh "$(top)/packaging/fedora/make-srpm.sh" "$(outdir)"
