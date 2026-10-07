#!/usr/bin/env bash
# Monta os pacotes .deb e .rpm do app nativo a partir do binário já compilado: abrem com dois cliques na loja de programas.
# Uso: tools/package-linux.sh <binário> <versão> <pasta-de-saída> <nome-base>
#   tools/package-linux.sh target/release/hangar-native 0.1.0.2533 out Hangar-linux-x86_64
set -euo pipefail
bin=$(realpath "$1"); version=$2; out=$(realpath "$3"); name=$4
here=$(cd "$(dirname "$0")/.." && pwd)
root=$(mktemp -d); trap 'rm -rf "$root"' EXIT
pkg="$root/pkg"
install -Dm755 "$bin" "$pkg/usr/bin/hangar-native"
install -Dm644 "$here/assets/brand/icon.png" "$pkg/usr/share/icons/hicolor/512x512/apps/com.hangar.native.png"
mkdir -p "$pkg/usr/share/applications"
# O mesmo .desktop do install-linux.sh, com o caminho do pacote.
printf '[Desktop Entry]\nType=Application\nName=Hangar\nExec=/usr/bin/hangar-native %%u\nIcon=com.hangar.native\nTerminal=false\nCategories=Development;\nMimeType=x-scheme-handler/hangar;\nStartupWMClass=com.hangar.native\n' \
  > "$pkg/usr/share/applications/com.hangar.native.desktop"

deb="$root/deb"
cp -a "$pkg" "$deb"
mkdir -p "$deb/DEBIAN"
cat > "$deb/DEBIAN/control" <<EOF
Package: hangar
Version: $version
Architecture: amd64
Maintainer: Hangar <jeffer1312@users.noreply.github.com>
Section: devel
Priority: optional
Homepage: https://hangar.dev.br
Description: Hangar desktop app
 Native desktop client for the Hangar server.
EOF
dpkg-deb --root-owner-group --build "$deb" "$out/$name.deb"

mkdir -p "$root/rpm/SPECS"
cat > "$root/rpm/SPECS/hangar.spec" <<EOF
%global debug_package %{nil}
%define _build_id_links none
Name: hangar
Version: $version
Release: 1
Summary: Hangar desktop app
License: MIT
URL: https://hangar.dev.br
%description
Native desktop client for the Hangar server.
%install
cp -a $pkg/. %{buildroot}/
%files
/usr/bin/hangar-native
/usr/share/applications/com.hangar.native.desktop
/usr/share/icons/hicolor/512x512/apps/com.hangar.native.png
EOF
rpmbuild -bb --define "_topdir $root/rpm" "$root/rpm/SPECS/hangar.spec"
cp "$root"/rpm/RPMS/x86_64/hangar-*.rpm "$out/$name.rpm"
