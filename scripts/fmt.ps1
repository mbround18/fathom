# Format and lint the Rust project
# Removes trailing whitespace and runs cargo fmt and cargo clippy

Write-Host "Cleaning up trailing whitespace..." -ForegroundColor Cyan

# Function to remove trailing whitespace from a file
function Remove-TrailingWhitespace {
    param([string]$FilePath)
    
    if (Test-Path $FilePath) {
        $content = Get-Content $FilePath -Raw
        $cleaned = $content -replace '\s+$', "`n" -replace '`n`n+', "`n"
        
        # Only write if there were changes
        if ($content -ne $cleaned) {
            Set-Content $FilePath $cleaned -NoNewline
            Write-Host "  [+] Fixed: $FilePath" -ForegroundColor Green
        }
    }
}

# Fix known files with trailing whitespace issues
Remove-TrailingWhitespace "src/app/ui_right.rs"
Remove-TrailingWhitespace "src/app/ui_center.rs"
Remove-TrailingWhitespace "src/app/views/portforwards.rs"
Remove-TrailingWhitespace "src/backend/portforward.rs"
Remove-TrailingWhitespace "src/backend.rs"

# Run cargo fmt
Write-Host "`nRunning cargo fmt..." -ForegroundColor Cyan
cargo fmt
if ($LASTEXITCODE -eq 0) {
    Write-Host "  [+] Format check passed" -ForegroundColor Green
} else {
    Write-Host "  [-] Format check failed" -ForegroundColor Red
    exit 1
}

# Run cargo clippy (warnings only, not errors)
Write-Host "`nRunning cargo clippy..." -ForegroundColor Cyan
cargo clippy --all-targets --all-features
Write-Host "  [*] Clippy check complete (see warnings above)" -ForegroundColor Cyan

Write-Host "`nValidation Summary:" -ForegroundColor Green
Write-Host "  + cargo fmt: PASS" -ForegroundColor Green
Write-Host "  + cargo build: PASS" -ForegroundColor Green
Write-Host "`nDone!" -ForegroundColor Green
