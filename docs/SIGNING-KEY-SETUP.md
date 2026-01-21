# Tauri Signing Key Setup Guide

**Critical:** This guide shows you how to generate, store, and configure the cryptographic signing keys required for LocalRouter AI's auto-update mechanism.

⚠️ **Security Warning:** Your private signing key is extremely sensitive. Anyone with access to it can create fraudulent updates for your app. Follow this guide carefully.

---

## Table of Contents

1. [Prerequisites](#prerequisites)
2. [Generate Signing Key](#step-1-generate-signing-key)
3. [Save to Bitwarden](#step-2-save-private-key-to-bitwarden)
4. [Add to GitHub Secrets](#step-3-add-private-key-to-github-secrets)
5. [Update tauri.conf.json](#step-4-update-tauriconfjson-with-public-key)
6. [Verify Setup](#step-5-verify-setup)
7. [Trigger First Release](#step-6-trigger-first-release)
8. [Troubleshooting](#troubleshooting)

---

## Prerequisites

**Required Tools:**
- Tauri CLI v2.x installed (`cargo install tauri-cli`)
- Bitwarden account (or other secure password manager)
- Bitwarden CLI (optional, for command-line export)
- GitHub account with admin access to LocalRouter repository
- macOS, Linux, or Windows with terminal access

**Check Tauri CLI version:**
```bash
cargo tauri --version
# Should show: tauri-cli 2.x.x
```

---

## Step 1: Generate Signing Key

### 1.1 Navigate to Project Root

```bash
cd /path/to/localrouterai
```

### 1.2 Generate the Key Pair

Run the Tauri signer command:

```bash
cargo tauri signer generate -w ~/.tauri/localrouter-signing.key
```

**What this does:**
- Creates a new cryptographic key pair (private + public)
- Saves the **private key** to `~/.tauri/localrouter-signing.key`
- Outputs the **public key** to your terminal

**Example Output:**
```
Private key saved to: /Users/yourusername/.tauri/localrouter-signing.key

Your public key (add to tauri.conf.json):
dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDMyRkQzNEJEQzRBNjk2RTYKUldSR0FBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQQo=
```

### 1.3 Copy Public Key

**IMMEDIATELY** copy the public key from the terminal output. You'll need it in Step 4.

Example public key format:
```
dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDMyRkQzNEJEQzRBNjk2RTYKUldSR0FBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQQo=
```

**Save this somewhere temporarily** (TextEdit, Notes app, etc.) - you'll need it in a moment.

---

## Step 2: Save Private Key to Bitwarden

The private key is now stored at `~/.tauri/localrouter-signing.key`. You MUST back it up securely.

### Option A: Manual Copy to Bitwarden (Recommended)

#### 2.1 Read the Private Key File

```bash
cat ~/.tauri/localrouter-signing.key
```

**Example Output:**
```
untrusted comment: rsign encrypted secret key
RWRTY5BndWKRrQqSVdDaRmG5tZ4F4+VwJYNGMQMKXxZ1234567890ABCDEFGHIJK
LMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/ABCDEFGHIJKL
MNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/
```

**⚠️ IMPORTANT:** Copy the **ENTIRE output**, including the "untrusted comment" line.

#### 2.2 Create Bitwarden Secure Note

1. **Open Bitwarden** (web vault or desktop app)
2. **Click "New Item"**
3. **Select type:** "Secure Note"
4. **Fill in details:**
   - **Name:** `LocalRouter Tauri Signing Key - Private`
   - **Notes:** Paste the ENTIRE private key (with "untrusted comment" line)
   - **Folder:** Choose appropriate folder (e.g., "Development Keys")
5. **Add Custom Field (Recommended):**
   - **Field Name:** `Generated Date`
   - **Value:** `2026-01-20` (today's date)
6. **Add Another Custom Field:**
   - **Field Name:** `Purpose`
   - **Value:** `LocalRouter AI auto-update signing (PRIVATE KEY - NEVER SHARE)`
7. **Click "Save"**

#### 2.3 Verify Bitwarden Entry

1. Close the secure note
2. Reopen it
3. Verify the key is intact and readable
4. **Make a backup of your Bitwarden vault** (export encrypted backup)

### Option B: Using Bitwarden CLI (Advanced)

```bash
# Install Bitwarden CLI if not installed
brew install bitwarden-cli  # macOS
# or download from: https://bitwarden.com/help/cli/

# Login
bw login

# Unlock vault and get session key
BW_SESSION=$(bw unlock --raw)

# Create secure note with private key
PRIVATE_KEY=$(cat ~/.tauri/localrouter-signing.key)
bw create item --session $BW_SESSION <<EOF
{
  "type": 2,
  "name": "LocalRouter Tauri Signing Key - Private",
  "notes": "$PRIVATE_KEY",
  "secureNote": {
    "type": 0
  }
}
EOF

# Lock vault
bw lock
```

---

## Step 3: Add Private Key to GitHub Secrets

GitHub Actions needs access to the private key to sign releases.

### 3.1 Read Private Key Again

```bash
cat ~/.tauri/localrouter-signing.key
```

**Copy the ENTIRE output** (including "untrusted comment" line).

### 3.2 Navigate to GitHub Repository Settings

1. Go to: **https://github.com/LocalRouter/LocalRouter**
2. Click **Settings** (top right, requires admin access)
3. In left sidebar, click **Secrets and variables** → **Actions**

### 3.3 Create TAURI_SIGNING_PRIVATE_KEY Secret

1. Click **"New repository secret"** (green button)
2. Fill in:
   - **Name:** `TAURI_SIGNING_PRIVATE_KEY`
   - **Secret:** Paste the ENTIRE private key (with "untrusted comment" line)
3. Click **"Add secret"**

**✅ Verification:** You should see `TAURI_SIGNING_PRIVATE_KEY` in the secrets list (value hidden).

### 3.4 Optional: Add Password Secret (If Key is Password-Protected)

If you generated a password-protected key, also add:

1. Click **"New repository secret"**
2. Fill in:
   - **Name:** `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
   - **Secret:** Your key password
3. Click **"Add secret"**

**Note:** The guide above generates an unencrypted key by default (no password). If you want a password-protected key, use:
```bash
cargo tauri signer generate -w ~/.tauri/localrouter-signing.key --password
# You'll be prompted to enter a password
```

---

## Step 4: Update tauri.conf.json with Public Key

### 4.1 Open tauri.conf.json

```bash
nano src-tauri/tauri.conf.json
# or use your preferred editor
```

### 4.2 Find the Placeholder

Look for this section around line 47:

```json
"plugins": {
  "updater": {
    "active": true,
    "endpoints": [
      "https://github.com/LocalRouter/LocalRouter/releases/latest/download/latest.json"
    ],
    "pubkey": "PLACEHOLDER_PUBLIC_KEY_REPLACE_AFTER_GENERATING",
    "windows": {
      "installMode": "passive"
    }
  }
}
```

### 4.3 Replace Placeholder with Public Key

Replace `"PLACEHOLDER_PUBLIC_KEY_REPLACE_AFTER_GENERATING"` with your **public key** from Step 1.3.

**Result:**
```json
"plugins": {
  "updater": {
    "active": true,
    "endpoints": [
      "https://github.com/LocalRouter/LocalRouter/releases/latest/download/latest.json"
    ],
    "pubkey": "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDMyRkQzNEJEQzRBNjk2RTYKUldSR0FBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQQo=",
    "windows": {
      "installMode": "passive"
    }
  }
}
```

### 4.4 Save and Commit

```bash
# Save the file (Ctrl+X, then Y if using nano)

# Verify the change
git diff src-tauri/tauri.conf.json

# Commit the change
git add src-tauri/tauri.conf.json
git commit -m "chore: add Tauri signing public key"
git push origin main
```

**⚠️ IMPORTANT:** Only commit the **public key**, never the private key!

---

## Step 5: Verify Setup

### 5.1 Check GitHub Secrets

1. Go to: **https://github.com/LocalRouter/LocalRouter/settings/secrets/actions**
2. Verify `TAURI_SIGNING_PRIVATE_KEY` exists (green checkmark)

### 5.2 Check tauri.conf.json

```bash
grep -A 10 '"updater"' src-tauri/tauri.conf.json
```

**Expected output:**
```json
"updater": {
  "active": true,
  "endpoints": [
    "https://github.com/LocalRouter/LocalRouter/releases/latest/download/latest.json"
  ],
  "pubkey": "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDMyRkQzNEJEQzRBNjk2RTYKUldSR0FBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQQo=",
  "windows": {
    "installMode": "passive"
  }
}
```

Verify `pubkey` is **NOT** `"PLACEHOLDER_PUBLIC_KEY_REPLACE_AFTER_GENERATING"`.

### 5.3 Check Local Key File

```bash
ls -lh ~/.tauri/localrouter-signing.key
```

**Expected:** File exists (~1-2KB)

### 5.4 Test Key Locally (Optional)

```bash
# Try signing a dummy file to verify key works
echo "test" > /tmp/test.txt
cargo tauri signer sign /tmp/test.txt -k ~/.tauri/localrouter-signing.key
# Should create test.txt.sig without errors
```

---

## Step 6: Trigger First Release

Now you're ready to create your first signed release!

### 6.1 Navigate to GitHub Actions

1. Go to: **https://github.com/LocalRouter/LocalRouter/actions**
2. Click **"Release"** workflow (left sidebar)
3. Click **"Run workflow"** (blue button, top right)

### 6.2 Fill in Workflow Inputs

**Input Form:**
- **Branch:** `main`
- **Version to release:** `0.0.2` (or `0.1.0` for first major release)
- **Mark as pre-release:** ☐ Unchecked (unless testing)

Click **"Run workflow"** (green button).

### 6.3 Monitor Workflow Progress

The workflow will:
1. ✅ Bump version in all files
2. ✅ Commit and push version bump
3. ✅ Create git tag (e.g., `v0.0.2`)
4. ✅ Build for macOS (Intel + Apple Silicon)
5. ✅ Build for Windows (x64)
6. ✅ Build for Linux (x64)
7. ✅ Sign all binaries
8. ✅ Create GitHub Release with artifacts

**Expected duration:** 15-30 minutes

### 6.4 Verify Release

1. Go to: **https://github.com/LocalRouter/LocalRouter/releases**
2. You should see: **v0.0.2** (or your version)
3. **Assets should include:**
   - `LocalRouter-AI_0.0.2_x64.dmg` (macOS Intel)
   - `LocalRouter-AI_0.0.2_aarch64.dmg` (macOS Apple Silicon)
   - `LocalRouter-AI_0.0.2_x64-setup.exe` (Windows)
   - `LocalRouter-AI_0.0.2_amd64.AppImage` (Linux)
   - `.sig` files for each (signatures)
   - `latest.json` (update manifest)

4. **Download `latest.json`** and verify it contains:
   - `version` field matching your release
   - `pub_date` with current timestamp
   - Platform-specific download URLs
   - Signatures

**Example `latest.json`:**
```json
{
  "version": "0.0.2",
  "pub_date": "2026-01-20T12:00:00Z",
  "platforms": {
    "darwin-x86_64": {
      "url": "https://github.com/LocalRouter/LocalRouter/releases/download/v0.0.2/LocalRouter-AI_0.0.2_x64.dmg.tar.gz",
      "signature": "dW50cnVzdGVk..."
    },
    "darwin-aarch64": {
      "url": "https://github.com/LocalRouter/LocalRouter/releases/download/v0.0.2/LocalRouter-AI_0.0.2_aarch64.dmg.tar.gz",
      "signature": "dW50cnVzdGVk..."
    },
    "windows-x86_64": {
      "url": "https://github.com/LocalRouter/LocalRouter/releases/download/v0.0.2/LocalRouter-AI_0.0.2_x64-setup.nsis.zip",
      "signature": "dW50cnVzdGVk..."
    },
    "linux-x86_64": {
      "url": "https://github.com/LocalRouter/LocalRouter/releases/download/v0.0.2/LocalRouter-AI_0.0.2_amd64.AppImage.tar.gz",
      "signature": "dW50cnVzdGVk..."
    }
  }
}
```

---

## Step 7: Test Auto-Update

### 7.1 Install Previous Version

1. Download and install `v0.0.1` (current version)
2. Launch the app

### 7.2 Trigger Update Check

1. Open **Preferences → Updates**
2. Click **"Check Now"**
3. Should detect `v0.0.2` available
4. Click **"Update Now"**
5. Watch progress bar
6. App should restart automatically
7. Verify app is now running `v0.0.2`

### 7.3 Check Tray Menu

1. After update detection, system tray should show:
   - **"🔔 Review new update"** menu item
2. Click it → should open Preferences → Updates

---

## Troubleshooting

### Issue: "Workflow failed at build step"

**Symptoms:** GitHub Actions workflow fails during `npm run tauri build`

**Solutions:**
1. **Check secret name:** Must be exactly `TAURI_SIGNING_PRIVATE_KEY` (case-sensitive)
2. **Check secret format:** Must include "untrusted comment" line
3. **View workflow logs:** Click on failed job → expand "Build Tauri app" step
4. **Common errors:**
   - `Invalid signing key format` → Re-copy private key with comment line
   - `Secret not found` → Verify secret exists in repository settings
   - `Permission denied` → Check repository access (must be admin)

### Issue: "Public key mismatch"

**Symptoms:** App fails to update with "Invalid signature" error

**Solutions:**
1. Verify public key in `tauri.conf.json` matches private key
2. Regenerate both keys if mismatch:
   ```bash
   cargo tauri signer generate -w ~/.tauri/localrouter-signing-NEW.key
   # Update tauri.conf.json with NEW public key
   # Update GitHub Secret with NEW private key
   # Trigger new release
   ```

### Issue: "latest.json not found at endpoint"

**Symptoms:** App can't check for updates (404 error)

**Solutions:**
1. Verify release was published (not draft)
2. Check release assets include `latest.json`
3. Verify endpoint URL in `tauri.conf.json`:
   ```
   https://github.com/LocalRouter/LocalRouter/releases/latest/download/latest.json
   ```
4. Test endpoint in browser (should download JSON file)

### Issue: "Workflow can't push to main"

**Symptoms:** Version bump step fails with permission error

**Solutions:**
1. **Enable workflow permissions:**
   - Go to: https://github.com/LocalRouter/LocalRouter/settings/actions
   - Scroll to "Workflow permissions"
   - Select: "Read and write permissions"
   - Check: "Allow GitHub Actions to create and approve pull requests"
   - Click "Save"

2. **Alternative:** Use Personal Access Token (PAT)
   - Create PAT: https://github.com/settings/tokens
   - Add as repository secret: `GH_PAT`
   - Update workflow to use: `token: ${{ secrets.GH_PAT }}`

### Issue: "Missing .sig files in release"

**Symptoms:** Release has binaries but no `.sig` signature files

**Solutions:**
1. Verify `TAURI_SIGNING_PRIVATE_KEY` secret is set correctly
2. Check workflow logs for signing errors
3. Verify `createUpdaterArtifacts: true` in `tauri.conf.json`
4. Re-run release workflow

### Issue: "Can't find private key file locally"

**Symptoms:** Can't access `~/.tauri/localrouter-signing.key`

**Solutions:**
1. **Check if file exists:**
   ```bash
   ls -lh ~/.tauri/
   ```
2. **Retrieve from Bitwarden:**
   - Open Bitwarden
   - Find "LocalRouter Tauri Signing Key - Private"
   - Copy contents
   - Save to file:
     ```bash
     nano ~/.tauri/localrouter-signing.key
     # Paste key contents
     # Save with Ctrl+X, Y
     chmod 600 ~/.tauri/localrouter-signing.key
     ```

### Issue: "Workflow takes too long"

**Symptoms:** Workflow runs for >1 hour

**Solutions:**
1. Check for stuck jobs (click "Cancel workflow run")
2. Verify all dependencies install correctly
3. Check runner status: https://www.githubstatus.com
4. Re-run workflow (sometimes transient issues)

---

## Security Best Practices

### ✅ DO:
- Store private key in Bitwarden or equivalent secure password manager
- Add private key to GitHub Secrets immediately after generation
- Backup your Bitwarden vault regularly
- Use strong password for Bitwarden
- Enable 2FA on GitHub account
- Rotate signing key annually (requires new releases)
- Keep `~/.tauri/*.key` files with `chmod 600` permissions

### ❌ DON'T:
- Commit private key to git (`.gitignore` prevents this)
- Share private key via email/Slack/Discord
- Store private key in unencrypted files
- Upload private key to cloud storage (Dropbox, Google Drive)
- Use same key for multiple projects
- Give private key access to contractors/third parties
- Post private key in GitHub issues or pull requests

---

## Key Rotation (Advanced)

If you need to rotate the signing key (e.g., compromised, annual rotation):

### 1. Generate New Key Pair
```bash
cargo tauri signer generate -w ~/.tauri/localrouter-signing-NEW.key
```

### 2. Update Bitwarden
- Edit existing secure note OR create new note with date suffix
- Replace old key with new key

### 3. Update GitHub Secret
- Go to repository settings → Secrets
- Edit `TAURI_SIGNING_PRIVATE_KEY`
- Replace with new private key

### 4. Update tauri.conf.json
- Replace `pubkey` value with new public key
- Commit and push

### 5. Create New Release
- All future releases will use new key
- Old releases remain signed with old key (still valid)

### 6. Deprecate Old Key
- After 30 days, delete old key file:
  ```bash
  rm ~/.tauri/localrouter-signing.key.old
  ```
- Keep old key in Bitwarden archive (for reference)

---

## Additional Resources

- **Tauri Updater Docs:** https://v2.tauri.app/plugin/updater/
- **Tauri Signing Guide:** https://v2.tauri.app/reference/cli/signer/
- **GitHub Actions Secrets:** https://docs.github.com/en/actions/security-guides/encrypted-secrets
- **Bitwarden CLI:** https://bitwarden.com/help/cli/
- **LocalRouter Update Plan:** `plan/2026-01-20-UPDATE-MECHANISM-COMPLETE.md`

---

## Quick Reference

### Commands Cheat Sheet

```bash
# Generate signing key
cargo tauri signer generate -w ~/.tauri/localrouter-signing.key

# View private key (for backup)
cat ~/.tauri/localrouter-signing.key

# View public key from tauri.conf.json
grep -A 1 '"pubkey"' src-tauri/tauri.conf.json

# Test signing locally
echo "test" > /tmp/test.txt
cargo tauri signer sign /tmp/test.txt -k ~/.tauri/localrouter-signing.key

# Check GitHub secret exists (requires gh CLI)
gh secret list | grep TAURI_SIGNING

# Trigger release workflow (requires gh CLI)
gh workflow run release.yml -f version=0.0.2

# List releases
gh release list

# Download latest.json
curl -L https://github.com/LocalRouter/LocalRouter/releases/latest/download/latest.json
```

---

**Document Version:** 1.0
**Last Updated:** 2026-01-20
**Maintainer:** LocalRouter AI Team

**Need help?** Open an issue: https://github.com/LocalRouter/LocalRouter/issues
