#!/usr/bin/env bash
set -euo pipefail

desktop_dir="$(cd "$(dirname "$0")/.." && pwd)"
icons_dir="$desktop_dir/src-tauri/icons"
composer_dir="$icons_dir/Sula.icon"
work_dir="$(mktemp -d /tmp/sula-iconcomposer.XXXXXX)"

cleanup() {
  rm -rf "$work_dir"
}
trap cleanup EXIT

(
  cd "$desktop_dir"
  pnpm tauri icon "$icons_dir/sula_master-v3.png" --output "$work_dir/tauri"
)

for filename in \
  32x32.png \
  128x128.png \
  128x128@2x.png \
  icon.ico \
  icon.png \
  StoreLogo.png \
  Square30x30Logo.png \
  Square44x44Logo.png \
  Square71x71Logo.png \
  Square89x89Logo.png \
  Square107x107Logo.png \
  Square142x142Logo.png \
  Square150x150Logo.png \
  Square284x284Logo.png \
  Square310x310Logo.png
do
  cp "$work_dir/tauri/$filename" "$icons_dir/$filename"
done

mkdir "$work_dir/output"
xcrun actool \
  --compile "$work_dir/output" \
  --platform macosx \
  --minimum-deployment-target 12.0 \
  --app-icon Sula \
  --output-partial-info-plist "$work_dir/info.plist" \
  --target-device mac \
  "$composer_dir" >/dev/null

cp "$work_dir/output/Sula.icns" "$icons_dir/icon.icns"
echo "Generated Tauri icons and Icon Composer macOS icon."
