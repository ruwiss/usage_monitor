#!/usr/bin/env bash
# Drop updater .sig files from a GitHub Release and rewrite the notes so
# people see one download per OS. Signatures stay inside latest.json.
set -euo pipefail

TAG="${1:-${GITHUB_REF_NAME:-}}"
if [[ -z "${TAG}" || "${TAG}" != v* ]]; then
  TAG="$(gh release view --json tagName --jq .tagName)"
fi

REPO="${GITHUB_REPOSITORY:-$(gh repo view --json nameWithOwner --jq .nameWithOwner)}"
BASE="https://github.com/${REPO}/releases/download/${TAG}"

mapfile -t NAMES < <(gh release view "${TAG}" --json assets --jq '.assets[].name' | sort)

deleted=0
for name in "${NAMES[@]}"; do
  if [[ "${name}" == *.sig ]]; then
    gh release delete-asset "${TAG}" "${name}" --yes
    deleted=$((deleted + 1))
  fi
done

if [[ "${deleted}" -gt 0 ]]; then
  mapfile -t NAMES < <(gh release view "${TAG}" --json assets --jq '.assets[].name' | sort)
fi

link() {
  local label="$1" file="$2"
  printf -- '- [%s](%s/%s)\n' "${label}" "${BASE}" "${file}"
}

find_one() {
  local pat="$1"
  local n
  for n in "${NAMES[@]}"; do
    if [[ "${n}" == ${pat} ]]; then
      printf '%s' "${n}"
      return 0
    fi
  done
  return 1
}

body=""
append() { body+="$1"$'\n'; }

append "## Download"
append ""
append "Pick **one** file. Skip \`.app.tar.gz\` and \`latest.json\` — those are for auto-update."
append ""

win_exe="$(find_one '*x64-setup.exe' || true)"
win_msi="$(find_one '*.msi' || true)"
if [[ -n "${win_exe}${win_msi}" ]]; then
  append "### Windows"
  [[ -n "${win_exe}" ]] && link "Installer (recommended)" "${win_exe}"
  [[ -n "${win_msi}" ]] && link "MSI" "${win_msi}"
  append ""
fi

mac_arm="$(find_one '*aarch64.dmg' || true)"
mac_x64="$(find_one '*x64.dmg' || true)"
if [[ -n "${mac_arm}${mac_x64}" ]]; then
  append "### macOS"
  [[ -n "${mac_arm}" ]] && link "Apple Silicon" "${mac_arm}"
  [[ -n "${mac_x64}" ]] && link "Intel" "${mac_x64}"
  append ""
  append "Ad-hoc signed (no Developer ID). First launch: right-click → **Open**. If Gatekeeper says the app is damaged:"
  append ""
  append '```bash'
  append 'xattr -cr "/Applications/Usage Monitor.app"'
  append '```'
  append ""
  append "Later updates clear quarantine themselves."
  append ""
fi

deb_amd="$(find_one '*amd64.deb' || true)"
deb_arm="$(find_one '*arm64.deb' || true)"
rpm_x64="$(find_one '*x86_64.rpm' || true)"
rpm_arm="$(find_one '*aarch64.rpm' || true)"
app_amd="$(find_one '*amd64.AppImage' || true)"
app_arm="$(find_one '*aarch64.AppImage' || true)"
if [[ -n "${deb_amd}${deb_arm}${rpm_x64}${rpm_arm}${app_amd}${app_arm}" ]]; then
  append "### Linux"
  [[ -n "${deb_amd}" ]] && link "Debian / Ubuntu (amd64)" "${deb_amd}"
  [[ -n "${deb_arm}" ]] && link "Debian / Ubuntu (arm64)" "${deb_arm}"
  [[ -n "${rpm_x64}" ]] && link "Fedora / RHEL (x86_64)" "${rpm_x64}"
  [[ -n "${rpm_arm}" ]] && link "Fedora / RHEL (aarch64)" "${rpm_arm}"
  [[ -n "${app_amd}" ]] && link "AppImage (x64)" "${app_amd}"
  [[ -n "${app_arm}" ]] && link "AppImage (ARM64)" "${app_arm}"
  append "- Arch: [\`packaging/arch/PKGBUILD\`](https://github.com/${REPO}/blob/main/packaging/arch/PKGBUILD)"
  append ""
fi

tmp="$(mktemp)"
printf '%s' "${body}" > "${tmp}"
gh release edit "${TAG}" --notes-file "${tmp}"
rm -f "${tmp}"

echo "tidied ${TAG}: removed ${deleted} .sig files, ${#NAMES[@]} assets left"
