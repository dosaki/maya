#!/bin/sh
# An Ubuntu VM (Multipass) for building and testing Maya's Linux port from
# a Mac. Usage: sh scripts/linux-vm.sh create|sync|run '<cmd>'|shell|destroy
set -eu
cd "$(dirname "$0")/.."
VM="${MAYA_VM:-maya-ubuntu}"
USAGE="usage: sh scripts/linux-vm.sh create|sync|run '<cmd>'|shell|destroy"
PKGS="build-essential curl git pkg-config libssl-dev cmake clang libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev patchelf tmux libnotify-bin speech-dispatcher pulseaudio-utils libasound2-dev xvfb weston imagemagick wmctrl libsecret-tools file"
case "${1:-}" in
  create)
    # A VM left by a create that failed part-way is reused, so create can
    # simply be run again.
    if multipass info "$VM" >/dev/null 2>&1; then
      echo "$VM exists; finishing its setup"
    else
      multipass launch 24.04 --name "$VM" --cpus 4 --memory 8G --disk 40G
    fi
    multipass exec "$VM" -- sudo apt-get update
    multipass exec "$VM" -- sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y $PKGS
    multipass exec "$VM" -- sh -c 'curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal'
    multipass exec "$VM" -- sh -c 'curl -fsSL https://deb.nodesource.com/setup_22.x | sudo -E bash - && sudo apt-get install -y nodejs && sudo corepack enable && corepack prepare pnpm@latest --activate'
    multipass exec "$VM" -- mkdir -p maya
    echo "created $VM; now: sh scripts/linux-vm.sh sync" ;;
  sync)
    # `multipass exec` truncates large stdin streams, so the tree is
    # staged to a local file and sent with `multipass transfer` instead
    # of piping into a remote `tar -xf -`.
    tmp="$(mktemp /tmp/maya-sync.XXXXXX.tar)"
    trap 'rm -f "$tmp"' EXIT
    COPYFILE_DISABLE=1 tar --exclude=./target --exclude=./node_modules --exclude='./dist*' --exclude=./vendor --exclude=./.superpowers --exclude=./src-tauri/binaries -cf "$tmp" .
    multipass transfer "$tmp" "$VM":/tmp/maya-sync.tar
    rm -f "$tmp"
    multipass exec "$VM" -- sh -c 'rm -rf maya && mkdir maya && tar -xf /tmp/maya-sync.tar -C maya && rm -f /tmp/maya-sync.tar'
    multipass exec "$VM" -- sh -c 'cd maya && pnpm install --frozen-lockfile >/dev/null'
    echo "synced to $VM:~/maya" ;;
  run)
    [ -n "${2:-}" ] || { echo "$USAGE" >&2; exit 2; }
    multipass exec "$VM" -- sh -lc "export PATH=\$HOME/.cargo/bin:\$PATH; cd maya && $2" ;;
  shell)
    multipass shell "$VM" ;;
  destroy)
    multipass delete "$VM" && multipass purge ;;
  *) echo "$USAGE" >&2; exit 2 ;;
esac
