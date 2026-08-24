param(
    [switch]$DryRun,
    [switch]$OpenReport,
    [ValidateSet('baseline', 'stage1_no_background_trees')]
    [string]$Policy = 'baseline',
    [ValidateSet('baseline', 'matched_ab')]
    [string]$MeasurementMode = 'baseline',
    [switch]$StartImmediately
)

$ErrorActionPreference = 'Stop'
$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptRoot '..\..\..')
$binaryPath = Join-Path $repoRoot 'target\debug\examples\local_capture.exe'
$reportPath = Join-Path $scriptRoot 'capture-report.mjs'
$runRoot = Join-Path $repoRoot ('target\capture-goal1-real-' + $Policy + '-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
$stopFile = Join-Path $runRoot 'stop.signal'
$captureStdout = Join-Path $runRoot 'capture.stdout.log'
$captureStderr = Join-Path $runRoot 'capture.stderr.log'

function Speak([string]$text) {
    Write-Host "`n>>> $text" -ForegroundColor Cyan
    try {
        Add-Type -AssemblyName System.Speech
        $speaker = [System.Speech.Synthesis.SpeechSynthesizer]::new()
        $speaker.Rate = 0
        $speaker.Speak($text)
        $speaker.Dispose()
    } catch {
        Write-Warning "Speech unavailable: $($_.Exception.Message)"
    }
}

function Wait-Seconds([int]$seconds) {
    $deadline = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 250 }
}

function Add-Marker([string]$phase, [string]$label) {
    $manifest = Get-Content -Raw -LiteralPath (Join-Path $runRoot 'run.json') | ConvertFrom-Json
    $marker = [ordered]@{
        schema_version = 1
        run_id = $manifest.run_id
        policy = $manifest.policy
        measurement_mode = $manifest.measurement_mode
        sequence = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() * 1000
        timestamp = [DateTime]::UtcNow.ToString('o')
        monotonic_ms = $null
        kind = 'scenario_marker'
        marker_id = $label
        phase = $phase
        label = $label
        expected_app = $null
        expected_window = $null
        expected_url = $null
        notes = 'timed real-app scenario runner; marker has no UI interaction'
    }
    Add-Content -LiteralPath (Join-Path $runRoot 'markers.jsonl') -Value ($marker | ConvertTo-Json -Compress)
}

function Run-Step([string]$label, [string]$instruction, [int]$seconds) {
    Add-Marker 'start' $label
    Speak $instruction
    Wait-Seconds $seconds
    Add-Marker 'end' $label
}

if (-not (Test-Path -LiteralPath $binaryPath)) {
    throw "Capture binary missing: $binaryPath. Run cargo build -p dystil-capture --example local_capture --features native,debug-capture first."
}

if ($DryRun) {
    Write-Host 'Dry run passed.'
    Write-Host "Capture binary: $binaryPath"
    Write-Host 'This script never opens, controls, or substitutes an application. It only speaks timed prompts and writes local markers.'
    exit 0
}

if (-not $StartImmediately) {
    Write-Host 'Prepare your real apps now: Gmail or another real mail client, Edge, and one desktop app. Focus the first app you want captured, then press Enter. Capture has not started yet.' -ForegroundColor Yellow
    Read-Host | Out-Null
}

New-Item -ItemType Directory -Path $runRoot | Out-Null
$capture = $null
try {
    $capture = Start-Process -FilePath $binaryPath -ArgumentList @('--text-only', '--diagnostics', '--policy', $Policy, '--measurement-mode', $MeasurementMode, '--stop-file', $stopFile, '--data-dir', $runRoot) -WindowStyle Hidden -PassThru -RedirectStandardOutput $captureStdout -RedirectStandardError $captureStderr
    for ($attempt = 0; $attempt -lt 80 -and -not (Test-Path -LiteralPath (Join-Path $runRoot 'run.json')); $attempt += 1) { Start-Sleep -Milliseconds 250 }
    if (-not (Test-Path -LiteralPath (Join-Path $runRoot 'run.json'))) { throw 'Capture harness did not initialize.' }

    Run-Step 'idle_70_seconds' 'Do not touch the mouse or keyboard for seventy seconds. This measures true idle behavior.' 70
    Run-Step 'ten_physical_clicks' 'In the real app now visible, make exactly ten deliberate clicks. Space them roughly one second apart.' 16
    Run-Step 'browser_navigation' 'Use Edge to navigate between two real pages or tabs. Pause briefly on each final page.' 20
    Run-Step 'email_threads' 'Use Gmail or your real mail app. Open three different threads, pausing briefly on each.' 24
    Run-Step 'control_change' 'Change one real dropdown, filter, or toggle, then pause. Do not make unrelated changes.' 10
    Run-Step 'long_scroll' 'Scroll continuously through real content for fifteen seconds, then stop and inspect the final region.' 20
    Run-Step 'typing_pause' 'Type a normal short paragraph in a real compose or editor field. Pause, edit one word, and pause again.' 18
    Run-Step 'app_switches' 'Switch among Edge, your mail app, and one desktop app several times. Leave the final app visible.' 16
    Run-Step 'continuous_activity' 'For twenty seconds, work normally with a mix of clicks, browsing, and short scrolls. Keep moving without a deliberate pause.' 20
    Run-Step 'final_idle' 'Stop interacting completely for twenty seconds.' 20

    New-Item -ItemType File -Path $stopFile | Out-Null
    $capture.WaitForExit(15000)
    if (-not $capture.HasExited) { throw 'Capture harness did not stop after the scenario.' }
    & bun $reportPath analyze --run-dir $runRoot
    if ($LASTEXITCODE -ne 0) { throw 'Report generation failed.' }
    Write-Host "`nScenario complete. Report: $(Join-Path $runRoot 'comparison.md')" -ForegroundColor Green
    if ($OpenReport) { Start-Process (Join-Path $runRoot 'comparison.md') }
} finally {
    if ($capture -and -not $capture.HasExited) {
        New-Item -ItemType File -Path $stopFile -Force | Out-Null
    }
}
