# Windows Chrome sandbox evidence

Read-only independent review: no production, test or CI sources were changed by this review. Initial Windows diagnostic CI37946708417 finished15 passed9 failed. All9 failed initial navigations correlate with sandbox executable READ/EXECUTE access denial and network-service restart. All9 later raw diagnostic navigations succeed with isDownload=false. The original failures remain failures.

Exact Chrome155 source requires AppContainer executable GENERIC_READ|GENERIC_EXECUTE at sandbox_win.cc804. CfT155's official win64 ZIP central directory confirms chrome-win64/setup.exe (7,324,672 bytes). Its official --configure-browser-in-directory flag accepts an absolute install directory and calls Chrome's capability-specific ACL helper. Installer enum success78/failure79 is verified from tag155.0.8059.39, not inferred from Puppeteer's ignored result. The helper grants inheritable read/execute to chromeInstallFiles and lpacChromeInstallFiles capability SIDs.

The root's reviewed workflow uses Start-Process -Wait -PassThru with one quoted option value, accepts only78, then checked icacls.exe inspection leaves PowerShell LASTEXITCODE0. reviewed-workflow.yml freezes the reviewed text; root may subsequently change its own workflow. Native Windows success after the setup correction is still required.

Authoritative URLs and fetched source SHA256 digests are in sources.json. chrome-win64-layout.json and range-metadata.json describe a TLS-verified HTTP206 suffix inspection. chrome-win64-central-directory-suffix.bin contains the exact received suffix. The sparse ZIP is a local seek aid, not a complete downloaded archive, and is excluded from the manifest/archive payload. It cannot establish authenticity of executable contents; it establishes the published ZIP entry.

review-report.json records raw evidence counts and the original Windows log digest. REVIEW.md records the detailed reasoning and limitations. checksum-manifest.sha256 freezes all evidence files except the manifest itself and the sparse seek aid. No SOTA or complete crossOS success is claimed.
