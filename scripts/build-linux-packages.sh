#!/bin/sh
# Builds native Linux packages from an unpacked release directory. Each
# package needs its own distribution's tooling (dpkg-deb on Debian/Ubuntu,
# rpmbuild on Fedora/RHEL), so the script builds whatever this host has and
# requires at least one; the release workflow runs it once in a Debian
# container for .deb and once in a Fedora container for .rpm.
#
#   scripts/build-linux-packages.sh <unpacked-dir> <out-dir> <version>
set -eu

STAGE="$1"        # the directory holding bin/diskgraph and bin/diskgraph-mcp
OUT="$2"          # where the .deb and .rpm are written
VERSION="$3"      # e.g. 0.2.1
NAME="diskgraph"
ARCH_DEB="amd64"
ARCH_RPM="x86_64"
SUMMARY="File-relationship engine for AI agents"
HOMEPAGE="https://github.com/loong10k/diskgraph"
# The license ships with the release; when this script runs from a checkout
# it is one level up, and when the release chain fetches it into a temp
# directory the working directory is the checkout instead.
LICENSE="$PWD/LICENSE"
if [ ! -f "$LICENSE" ]; then
  LICENSE="$(cd "$(dirname "$0")/.." && pwd)/LICENSE"
fi

test -x "$STAGE/bin/diskgraph" || { echo "no staged binary at $STAGE/bin/diskgraph" >&2; exit 1; }
test -f "$LICENSE" || { echo "LICENSE not found" >&2; exit 1; }
mkdir -p "$OUT"

# The source tree: /usr/bin binaries, the license under the doc dir.
build_tree() {
  root="$1"
  rm -rf "$root"
  mkdir -p "$root/DEBIAN" "$root/usr/bin" "$root/usr/share/doc/$NAME"
  cp "$STAGE/bin/diskgraph" "$STAGE/bin/diskgraph-mcp" "$root/usr/bin/"
  chmod 0755 "$root/usr/bin/diskgraph" "$root/usr/bin/diskgraph-mcp"
  cp "$LICENSE" "$root/usr/share/doc/$NAME/LICENSE"
  cp "$STAGE/README.md" "$root/usr/share/doc/$NAME/README.md" 2>/dev/null || true
}

built=0

# ---- deb -------------------------------------------------------------------
if command -v dpkg-deb >/dev/null 2>&1; then
build_tree "$OUT/deb-root"
cat > "$OUT/deb-root/DEBIAN/control" <<EOF
Package: $NAME
Version: $VERSION
Section: utils
Priority: optional
Architecture: $ARCH_DEB
Maintainer: DiskGraph contributors <noreply@github.com>
Homepage: $HOMEPAGE
Description: $SUMMARY
 Indexes directories into a local graph so agents can answer bounded
 questions about disk usage, ownership, evidence, and history without
 walking the filesystem again.
EOF
# DiskGraph has no library dependencies beyond libc and the C runtime the
# bundled SQLite needs; both are part of any base system.
cat > "$OUT/deb-root/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
# The engine stores both databases in a 0700 data directory it creates on
# first use; nothing to do at install time beyond leaving the binaries in
# place.
exit 0
EOF
chmod 0755 "$OUT/deb-root/DEBIAN/postinst"
dpkg-deb --build --root-owner-group "$OUT/deb-root" "$OUT/${NAME}_${VERSION}_${ARCH_DEB}.deb"
built=$((built + 1))
fi

# ---- rpm -------------------------------------------------------------------
if command -v rpmbuild >/dev/null 2>&1; then
RPM_TOP="$OUT/rpmbuild"
build_tree "$RPM_TOP/BUILDROOT"
mkdir -p "$RPM_TOP/SPECS" "$RPM_TOP/RPMS" "$RPM_TOP/SRPMS"
cat > "$RPM_TOP/SPECS/$NAME.spec" <<EOF
Name:           $NAME
Version:        $VERSION
Release:        1
Summary:        $SUMMARY
License:        MIT
URL:            $HOMEPAGE
BuildArch:      $ARCH_RPM

%description
Indexes directories into a local graph so agents can answer bounded
questions about disk usage, ownership, evidence, and history without
walking the filesystem again.

%prep
%build
%install
mkdir -p %{buildroot}/usr/bin %{buildroot}/usr/share/doc/$NAME
cp __STAGE__/bin/diskgraph __STAGE__/bin/diskgraph-mcp %{buildroot}/usr/bin/
chmod 0755 %{buildroot}/usr/bin/diskgraph %{buildroot}/usr/bin/diskgraph-mcp
cp __LICENSE__ %{buildroot}/usr/share/doc/$NAME/LICENSE
cp __README__ %{buildroot}/usr/share/doc/$NAME/README.md 2>/dev/null || true

%files
/usr/bin/diskgraph
/usr/bin/diskgraph-mcp
/usr/share/doc/$NAME

%changelog
* Mon Jan 01 2024 DiskGraph contributors <noreply@github.com> - $VERSION-1
- Automated package build for the $VERSION release.
EOF
# Substitute the host paths rpmbuild cannot see.
sed -i \
  -e "s|__STAGE__|$STAGE|g" \
  -e "s|__LICENSE__|$LICENSE|g" \
  -e "s|__README__|$STAGE/README.md|g" \
  "$RPM_TOP/SPECS/$NAME.spec"
rpmbuild --define "_topdir $RPM_TOP" \
         --define "_arch $ARCH_RPM" \
         -bb "$RPM_TOP/SPECS/$NAME.spec" >/dev/null
find "$RPM_TOP/RPMS" -name '*.rpm' -exec cp {} "$OUT/" \;
built=$((built + 1))
fi

if [ "$built" -eq 0 ]; then
  echo "neither dpkg-deb nor rpmbuild is available on this host" >&2
  exit 1
fi

mv "$OUT"/*.fc*.rpm "$OUT"/ 2>/dev/null || true
echo "packages:"
ls -1 "$OUT"/*.deb "$OUT"/*.rpm 2>/dev/null
