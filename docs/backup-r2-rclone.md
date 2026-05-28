# Backup Jottrace to Cloudflare R2 (rclone + encryption)

Daily workflow:

1. `jottrace pack` → a temporary `.tar.gz` on disk.
2. `rclone` uploads through an encrypted **`crypt`** remote into **R2**.
3. A retention step keeps the **30 newest** dated archives on the remote.

Restore path: download an archive from R2 → `jottrace settle <archive>` (see [Restore](#restore-drill)).

**Do not** sync or upload the live `~/.jottrace` directory while Jottrace is writing. Only upload archives produced by `pack` (WAL is checkpointed during pack).

---

## Legend

| Label | Meaning |
|-------|---------|
| **[You]** | Only you can do this (cloud console, secrets, scheduling on your Mac, one-off tests on your machine). |
| **[Agent]** | A coding agent can do this in the repo or workspace when you ask (add scripts, example configs, doc edits). |
| **[You + Agent]** | Agent prepares files; you install paths, secrets, and run `launchctl` locally. |

---

## Prerequisites

| Step | Owner | Action |
|------|-------|--------|
| Jottrace installed and journal exists | **[You]** | `jottrace --version`, `jottrace doctor`, `jottrace status` |
| `rclone` installed | **[You]** | e.g. `brew install rclone` |
| macOS user account for daily scheduling | **[You]** | This guide uses **launchd** (adjust if you use Linux cron instead) |

---

## 1. Cloudflare R2

| Step | Owner | Action |
|------|-------|--------|
| Create an R2 bucket | **[You]** | Dashboard → **R2** → create bucket (example name: `YOUR_BUCKET_NAME`) |
| Create API token | **[You]** | **Manage R2 API tokens** → token with **Object Read & Write** on that bucket |
| Save credentials | **[You]** | Access Key ID, Secret Access Key, Account ID (password manager) |
| Note S3 endpoint | **[You]** | `https://<ACCOUNT_ID>.r2.cloudflarestorage.com` |

---

## 2. rclone remotes (R2 + encryption)

Run once on your Mac:

```sh
rclone config
```

### 2a. Remote `r2` (S3-compatible → R2)

| Prompt | Value |
|--------|--------|
| Name | `r2` |
| Storage | `s3` |
| Provider | `Cloudflare` |
| access_key_id / secret_access_key | From R2 token |
| endpoint | `https://<ACCOUNT_ID>.r2.cloudflarestorage.com` |
| acl | `private` |

| Step | Owner | Action |
|------|-------|--------|
| Run `rclone config` and create `r2` | **[You]** | Interactive; secrets stay on your machine |
| Test listing | **[You]** | `rclone lsd r2:` (adjust if your config names the bucket differently) |

### 2b. Remote `r2crypt` (encryption wrapper)

| Prompt | Value |
|--------|--------|
| Name | `r2crypt` |
| Storage | `crypt` |
| remote | `r2:YOUR_BUCKET_NAME` (bucket + optional prefix; match your bucket name) |
| filename_encryption | `standard` |
| directory_name_encryption | `true` |
| password | Strong password (save in password manager) |

| Step | Owner | Action |
|------|-------|--------|
| Create `r2crypt` remote | **[You]** | Interactive |
| Test encrypt round-trip | **[You]** | See commands below |

```sh
echo "test" | rclone rcat r2crypt:_probe.txt
rclone cat r2crypt:_probe.txt
rclone delete r2crypt:_probe.txt
```

| Step | Owner | Action |
|------|-------|--------|
| Protect `~/.config/rclone/rclone.conf` | **[You]** | `chmod 600`; back up config + remember crypt password |
| Optional: store crypt password for launchd | **[You]** | Keychain, or `RCLONE_CONFIG_R2CRYPT_PASS` in plist (see §5) |

Config file path: `~/.config/rclone/rclone.conf` — **never commit this file to git.**

---

## 3. Backup script

The script: pack to a temp file → upload with a dated name → delete oldest remote files until **30** remain.

| Step | Owner | Action |
|------|-------|--------|
| Add `scripts/jottrace-backup-to-r2` to the repo | **[Agent]** | When you ask the agent to add it |
| Install script to `~/.local/bin/` (or edit paths) | **[You]** | Copy or symlink; `chmod 700` |
| Set `REMOTE`, `KEEP`, log path if needed | **[You]** or **[Agent]** | Agent can use placeholders; you set real remote name |
| Fix full paths to `jottrace` / `rclone` in script or plist | **[You]** | If launchd PATH is minimal |

### Script contents (reference)

Save as `~/.local/bin/jottrace-backup-to-r2` (or use the repo copy under `scripts/` once added):

```sh
#!/usr/bin/env bash
set -euo pipefail

REMOTE="r2crypt:"
KEEP=30
LOG="${HOME}/Library/Logs/jottrace-backup.log"

exec >>"$LOG" 2>&1
echo "=== $(date -u +%Y-%m-%dT%H:%M:%SZ) ==="

ARCHIVE="$(mktemp -t jottrace-pack)"
ARCHIVE="${ARCHIVE}.tar.gz"

cleanup() { rm -f "$ARCHIVE"; }
trap cleanup EXIT

jottrace pack --output "$ARCHIVE"

BASENAME="jottrace-pack-$(date -u +%Y%m%d-%H%M%SZ).tar.gz"

rclone copyto "$ARCHIVE" "${REMOTE}${BASENAME}" --s3-no-check-bucket

mapfile -t files < <(rclone lsf "$REMOTE" --files-only | sort)
count=${#files[@]}
if (( count > KEEP )); then
  delete_count=$((count - KEEP))
  for ((i = 0; i < delete_count; i++)); do
    rclone deletefile "${REMOTE}${files[i]}"
    echo "deleted ${files[i]}"
  done
fi

echo "done: uploaded ${BASENAME}, remote count<=${KEEP}"
```

| Step | Owner | Action |
|------|-------|--------|
| `chmod 700 ~/.local/bin/jottrace-backup-to-r2` | **[You]** | |
| First manual run | **[You]** | `~/.local/bin/jottrace-backup-to-r2` |
| Check log | **[You]** | `tail -30 ~/Library/Logs/jottrace-backup.log` |
| Check remote count | **[You]** | `rclone lsf r2crypt: \| sort` (should show ≤30 after several days) |

**Non-default journal directory:** if you use `JOTTRACE_HOME`, export it in the script or in the launchd plist before `jottrace pack`.

**Schedule timing:** `pack` uses the same data lock as `ingest`. Avoid overlapping a long ingest if possible; early morning is a reasonable default.

---

## 4. Restore drill

| Step | Owner | Action |
|------|-------|--------|
| Download one archive | **[You]** | `rclone copy r2crypt:jottrace-pack-YYYYMMDD-HHMMSSZ.tar.gz ~/jottrace-restore-test/` |
| Restore into a test directory | **[You]** | See below |
| Confirm `jottrace status` | **[You]** | |

```sh
mkdir -p ~/jottrace-restore-test
rclone copy r2crypt:jottrace-pack-YYYYMMDD-HHMMSSZ.tar.gz ~/jottrace-restore-test/

export JOTTRACE_HOME=~/jottrace-restore-test/journal
mkdir -p "$JOTTRACE_HOME"
jottrace settle --force ~/jottrace-restore-test/jottrace-pack-YYYYMMDD-HHMMSSZ.tar.gz
jottrace status --details
```

On your real machine (overwrite existing journal), use your normal `JOTTRACE_HOME` and **`settle --force` only when you intend to replace** the local journal.

| Step | Owner | Action |
|------|-------|--------|
| Document which archive / date you restored | **[You]** | For your own runbook |

---

## 5. Daily schedule (macOS launchd)

| Step | Owner | Action |
|------|-------|--------|
| Add example plist to repo (e.g. `scripts/com.example.jottrace.backup.plist.example`) | **[Agent]** | When you ask |
| Copy plist to `~/Library/LaunchAgents/` | **[You]** | Edit username paths |
| Set `Hour` / `Minute` | **[You]** | Default example: 04:00 local |
| Add `PATH`, `JOTTRACE_HOME`, `RCLONE_CONFIG_R2CRYPT_PASS` if needed | **[You]** | Secrets stay on your Mac |
| `launchctl bootstrap` / `kickstart` | **[You]** | Commands below |

Example plist (`~/Library/LaunchAgents/com.example.jottrace.backup.plist`) — **edit paths and username**:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.example.jottrace.backup</string>
  <key>ProgramArguments</key>
  <array>
    <string>/Users/YOUR_USERNAME/.local/bin/jottrace-backup-to-r2</string>
  </array>
  <key>StartCalendarInterval</key>
  <dict>
    <key>Hour</key>
    <integer>4</integer>
    <key>Minute</key>
    <integer>0</integer>
  </dict>
  <key>StandardOutPath</key>
  <string>/Users/YOUR_USERNAME/Library/Logs/jottrace-backup.launchd.out.log</string>
  <key>StandardErrorPath</key>
  <string>/Users/YOUR_USERNAME/Library/Logs/jottrace-backup.launchd.err.log</string>
</dict>
</plist>
```

Optional environment (uncomment and edit in your real plist):

```xml
<key>EnvironmentVariables</key>
<dict>
  <key>RCLONE_CONFIG_R2CRYPT_PASS</key>
  <string>YOUR_CRYPT_PASSWORD</string>
  <key>PATH</key>
  <string>/opt/homebrew/bin:/usr/local/bin:/Users/YOUR_USERNAME/.local/bin:/usr/bin:/bin</string>
  <key>JOTTRACE_HOME</key>
  <string>/Users/YOUR_USERNAME/.jottrace</string>
</dict>
```

| Step | Owner | Action |
|------|-------|--------|
| Load agent | **[You]** | |

```sh
launchctl bootout gui/$(id -u) ~/Library/LaunchAgents/com.example.jottrace.backup.plist 2>/dev/null || true
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.example.jottrace.backup.plist
launchctl enable gui/$(id -u)/com.example.jottrace.backup
launchctl kickstart -k gui/$(id -u)/com.example.jottrace.backup
```

| Step | Owner | Action |
|------|-------|--------|
| Confirm next runs in logs | **[You]** | Check logs after kickstart and the next day |

---

## 6. Ongoing maintenance

| Task | Owner | Frequency |
|------|-------|-----------|
| Skim `~/Library/Logs/jottrace-backup.log` for failures | **[You]** | Weekly |
| Confirm remote has ≤30 objects | **[You]** | After first month |
| Rotate R2 API token if compromised | **[You]** | As needed |
| Re-run restore drill after major jottrace upgrade | **[You]** | Optional |
| Update script/plist when install paths change | **[You]** or **[Agent]** | As needed |

---

## 7. What not to use for this workflow

| Approach | Why |
|----------|-----|
| Sync live `~/.jottrace` with Drive/iCloud | WAL + lock → corrupt or inconsistent backups |
| R2 lifecycle “delete after 30 days” only | That is **30 days**, not **30 archives** |
| Commit `rclone.conf` or API keys to git | Secrets leak |

---

## Quick checklist

- [ ] **[You]** R2 bucket + API token  
- [ ] **[You]** `rclone` remotes `r2` + `r2crypt` tested  
- [ ] **[You + Agent]** Backup script installed (`scripts/` optional via agent)  
- [ ] **[You]** Manual backup succeeded; log looks good  
- [ ] **[You]** Restore drill to a test `JOTTRACE_HOME`  
- [ ] **[You]** launchd plist loaded; kickstart succeeded  
- [ ] **[You]** After 31+ runs, remote retains at most 30 archives  

---

## Related Jottrace commands

| Command | Role |
|---------|------|
| `jottrace pack --output <path>` | Consistent archive (WAL checkpoint + lock) |
| `jottrace settle <archive>` | Restore journal from archive |
| `jottrace doctor` / `status` | Verify local journal health |

See also: [README.md](../README.md) (pack/settle section), [design.md](design.md) (storage overview).
