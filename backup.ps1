# =============================
# ADB Backup Utility - PowerShell 7 (Windows)
# Fully handles spaces, TAR backup/restore
# =============================

function Check-Device {
    $dev = & adb devices | Select-String "device$"
    if (-not $dev) {
        Write-Host "`n[ERROR] No Android device detected." -ForegroundColor Red
        return $false
    }
    return $true
}

function Create-TimestampFile($prefix, $ext) {
    $timestamp = Get-Date -Format "yyyyMMdd_HHmmss"
    return "$prefix`_$timestamp.$ext"
}

function Create-TimestampFolder($prefix) {
    $timestamp = Get-Date -Format "yyyyMMdd_HHmmss"
    $folder = "$prefix`_$timestamp"
    New-Item -ItemType Directory -Path $folder -Force | Out-Null
    return $folder
}

function Check-FolderOnPhone($path) {
    $exists = adb shell "[ -d '$path' ] && echo EXISTS"
    return $exists -eq "EXISTS"
}

# ============================================================
# Pac-Man animation
# ============================================================
function Show-PacManDynamic {
    param (
        [int]$current,
        [int]$total,
        [int]$barLength = 30
    )
    $percent = [math]::Round(($current / $total) * 100)
    $pacPos = [math]::Floor(($current / $total) * $barLength)
    if ($pacPos -ge $barLength) { $pacPos = $barLength - 1 }

    $bar = ""
    for ($i=0; $i -lt $barLength; $i++) {
        if ($i -eq $pacPos) { $bar += "C" }
        elseif ($i -gt $pacPos) { $bar += "o" }
        else { $bar += " " }
    }
    Write-Host -NoNewline "`r[$bar] $percent% ($current/$total files) "
}

# ============================================================
# WhatsApp DB Backup
# ============================================================
function Backup-WhatsAppDB {
    if (-not (Check-Device)) { return }

    $pathsToCheck = @(
        "/sdcard/Android/media/com.whatsapp/WhatsApp/Databases",
        "/storage/emulated/0/WhatsApp/Databases"
    )

    $waPath = $null
    foreach ($p in $pathsToCheck) {
        if (Check-FolderOnPhone $p) { $waPath = $p; break }
    }

    if (-not $waPath) {
        Write-Host "[ERROR] WhatsApp DB folder not found or blocked." -ForegroundColor Red
        return
    }

    Write-Host "[INFO] Found WhatsApp DB folder: $waPath"

    $backupDir = Create-TimestampFolder "Backup_WhatsAppDB"
    Write-Host "Backing up WhatsApp DB to $backupDir..."

    $files = adb shell "ls -1 '$waPath'" | ForEach-Object { $_.Trim() }
    $total = $files.Count
    if ($total -eq 0) { Write-Host "[ERROR] No files found in $waPath" -ForegroundColor Red; return }

    $i = 0
    foreach ($f in $files) {
        $i++
        Show-PacManDynamic -current $i -total $total
        adb pull "'$waPath/$f'" "$backupDir" | Out-Null
        Start-Sleep -Milliseconds 50
    }
    Write-Host "`n"

    $localCount = (Get-ChildItem -Path $backupDir -File).Count
    if ($localCount -eq $total) {
        Write-Host "[OK] WhatsApp DB backup complete and verified!" -ForegroundColor Green
    } else {
        Write-Host "[WARNING] File count mismatch! Device: $total, Local: $localCount" -ForegroundColor Yellow
    }
}

# ============================================================
# WhatsApp DB Restore
# ============================================================
function Restore-WhatsAppDB {
    if (-not (Check-Device)) { return }

    $pathsToCheck = @(
        "/sdcard/Android/media/com.whatsapp/WhatsApp/Databases",
        "/storage/emulated/0/WhatsApp/Databases"
    )

    $waPath = $null
    foreach ($p in $pathsToCheck) {
        if (Check-FolderOnPhone $p) { $waPath = $p; break }
    }

    if (-not $waPath) {
        Write-Host "[ERROR] WhatsApp DB folder not found or blocked." -ForegroundColor Red
        return
    }

    Write-Host "[INFO] Restoring WhatsApp DB to: $waPath"
    Write-Host "`nEnter LOCAL folder containing WhatsApp database files:"
    $localPath = Read-Host

    if (-not (Test-Path $localPath)) {
        Write-Host "[ERROR] Local folder does not exist." -ForegroundColor Red
        return
    }

    $files = Get-ChildItem -Path $localPath -File
    $total = $files.Count
    if ($total -eq 0) { Write-Host "[ERROR] No files found in local folder." -ForegroundColor Red; return }

    $i = 0
    foreach ($f in $files) {
        $i++
        Show-PacManDynamic -current $i -total $total
        adb push "'$($f.FullName)'" "'$waPath'" | Out-Null
        Start-Sleep -Milliseconds 50
    }
    Write-Host "`n"

    $deviceCount = adb shell "ls -1 '$waPath' | wc -l" | ForEach-Object { $_.Trim() }
    if ($deviceCount -eq $total) {
        Write-Host "[OK] Restore complete and verified!" -ForegroundColor Green
    } else {
        Write-Host "[WARNING] File count mismatch! Local: $total, Device: $deviceCount" -ForegroundColor Yellow
    }
}

# ============================================================
# DCIM TAR Backup (PowerShell 7)
# ============================================================
function Backup-DCIM-Tar {
    if (-not (Check-Device)) { return }

    $dcimPath = "/sdcard/DCIM"
    $folders = adb shell "ls -d '$dcimPath'/*/" | ForEach-Object { $_.Trim() -replace '/sdcard/DCIM/', '' -replace '/','' }
    if (-not $folders) { Write-Host "[ERROR] No folders found." -ForegroundColor Red; return }

    Write-Host "`nDetected DCIM folders:"
    for ($i=0; $i -lt $folders.Count; $i++) { Write-Host "$($i+1)) $($folders[$i])" }

    $choice = Read-Host "Enter folder number to backup"
    if (-not [int]::TryParse($choice,[ref]$null) -or $choice -lt 1 -or $choice -gt $folders.Count) {
        Write-Host "[ERROR] Invalid selection!" -ForegroundColor Red
        return
    }

    $selectedFolder = $folders[$choice - 1]
    $remoteFolderPath = "$dcimPath/$selectedFolder"
    $backupFile = Create-TimestampFile "DCIM_$selectedFolder" "tar"

    # Count files recursively
    $fileList = adb shell "find '$remoteFolderPath' -type f" | ForEach-Object { $_.Trim() }
    $total = $fileList.Count
    if ($total -eq 0) { Write-Host "[ERROR] No files found." -ForegroundColor Red; return }
    Write-Host "[INFO] Files on device: $total"

    Write-Host "`nBacking up /sdcard/DCIM/$selectedFolder to $backupFile ..."

    # Correct binary TAR creation by using cmd.exe for redirection to handle raw byte stream
    $command = "adb exec-out ""tar -C '$remoteFolderPath' -cf - . 2>/dev/null"" > ""$backupFile"""
    cmd /c $command

    # Pac-Man progress
    for ($i=1; $i -le $total; $i++) {
        Show-PacManDynamic -current $i -total $total
        Start-Sleep -Milliseconds 50
    }
    Write-Host "`n"

    # Verification
    $tarCount = (& tar -tf $backupFile | Measure-Object).Count
    Write-Host "[INFO] Files in TAR archive: $tarCount"
    if ($tarCount -eq $total) {
        Write-Host "[OK] DCIM TAR backup complete and verified!" -ForegroundColor Green
    } else {
        Write-Host "[WARNING] File count mismatch!" -ForegroundColor Yellow
    }

    Write-Host "Saved as: $backupFile"
}

# ============================================================
# Restore TAR Backup (PowerShell 7)
# ============================================================
function Restore-TarBackup {
    if (-not (Check-Device)) { return }

    # Find .tar files in the current directory
    $tarFiles = Get-ChildItem -Path . -Filter "*.tar" | Sort-Object LastWriteTime -Descending
    if (-not $tarFiles) {
        Write-Host "`n[ERROR] No .tar files found in the current directory." -ForegroundColor Red
        return
    }

    Write-Host "`nDetected TAR backup files:"
    for ($i=0; $i -lt $tarFiles.Count; $i++) {
        Write-Host "$($i+1)) $($tarFiles[$i].Name)"
    }

    $choice = Read-Host "Enter file number to restore"
    if (-not [int]::TryParse($choice,[ref]$null) -or $choice -lt 1 -or $choice -gt $tarFiles.Count) {
        Write-Host "[ERROR] Invalid selection!" -ForegroundColor Red
        return
    }

    $selectedFile = $tarFiles[$choice - 1]
    $tarFile = $selectedFile.FullName
    $tarFileName = $selectedFile.Name

    # Determine destination path from backup filename
    $destPath = "/sdcard/RestoredContent" # Default path
    $match = [regex]::Match($tarFileName, '^DCIM_(.+)_\d{8}_\d{6}\.tar$')
    if ($match.Success) {
        $folderName = $match.Groups[1].Value
        $destPath = "/sdcard/DCIM/$folderName"
    } else {
        Write-Host "[WARNING] Could not determine original folder from filename. Using default: $destPath" -ForegroundColor Yellow
    }

    Write-Host "`n[INFO] TAR file to restore: $tarFile"
    Write-Host "[INFO] Destination on device: $destPath"

    if (-not (Test-Path $tarFile)) { Write-Host "`n[ERROR] TAR file not found at '$tarFile'." -ForegroundColor Red; return }

    Write-Host "`nRestoring TAR backup to device..."
    $tempTar = "/sdcard/temp_restore.tar"
    adb push "$tarFile" "$tempTar" | Out-Null

    $totalFiles = (& tar -tvf $tarFile | Where-Object { -not $_.StartsWith('d') }).Count
    if ($totalFiles -eq 0) {
        Write-Host "[WARNING] TAR file appears to be empty. Nothing to restore." -ForegroundColor Yellow
        adb shell "rm '$tempTar'"
        return
    }

    # Create destination folder and start extraction in background
    adb shell "mkdir -p '$destPath'; nohup tar -xf '$tempTar' -C '$destPath' >/dev/null 2>&1 &"

    # Real progress bar by polling file count
    $timeoutSeconds = 300 # 5-minute timeout
    $startTime = Get-Date
    $currentFiles = 0
    while ($currentFiles -lt $totalFiles) {
        $elapsed = (Get-Date) - $startTime
        if ($elapsed.TotalSeconds -gt $timeoutSeconds) {
            Write-Host "`n[ERROR] Restore operation timed out." -ForegroundColor Red
            break
        }

        $currentFiles = adb shell "find '$destPath' -type f 2>/dev/null | wc -l" | ForEach-Object { $_.Trim() }
        if (-not $currentFiles) { $currentFiles = 0 }

        Show-PacManDynamic -current $currentFiles -total $totalFiles
        Start-Sleep -Milliseconds 500
    }
    Show-PacManDynamic -current $totalFiles -total $totalFiles # Show 100%
    Write-Host "`n"

    # Cleanup
    adb shell "rm '$tempTar'"

    # Final verification
    $deviceCount = adb shell "find '$destPath' -type f 2>/dev/null | wc -l" | ForEach-Object { $_.Trim() }
    if (-not $deviceCount) { $deviceCount = 0 }

    if ($deviceCount -eq $totalFiles) {
        Write-Host "[OK] TAR restore complete and verified!" -ForegroundColor Green
    } else {
        Write-Host "[WARNING] File count mismatch! TAR: $totalFiles, Device: $deviceCount" -ForegroundColor Yellow
    }
}

# ============================================================
# MAIN MENU
# ============================================================
while ($true) {
    Write-Host ""
    Write-Host "=============================="
    Write-Host "     ADB Backup Utility"
    Write-Host "=============================="
    Write-Host "1) Backup WhatsApp Database"
    Write-Host "2) Restore WhatsApp Database"
    Write-Host "3) Fast TAR backup DCIM (Select Folder)"
    Write-Host "4) Restore TAR backup to Phone"
    Write-Host "0) Exit"
    Write-Host "=============================="

    $choice = Read-Host "Choose an option"
    switch ($choice) {
        1 { Backup-WhatsAppDB }
        2 { Restore-WhatsAppDB }
        3 { Backup-DCIM-Tar }
        4 { Restore-TarBackup }
        0 { exit }
        default { Write-Host "[ERROR] Invalid choice." -ForegroundColor Yellow }
    }
}
