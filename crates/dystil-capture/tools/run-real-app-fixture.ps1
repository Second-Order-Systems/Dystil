param(
    [ValidateSet('baseline', 'stage1_no_background_trees', 'stage2_click_coalesce', 'stage3_settled_state', 'stage4_visible_relevant')]
    [string]$Policy = 'stage3_settled_state',
    [ValidateSet('baseline', 'matched_ab')]
    [string]$MeasurementMode = 'matched_ab',
    [ValidateRange(1, 180)]
    [int]$IdleSeconds = 70,
    [switch]$DryRun,
    [switch]$OpenReport,
    [switch]$KeepFixtureApps
)

$ErrorActionPreference = 'Stop'
$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptRoot '..\..\..')
$binaryPath = Join-Path $repoRoot 'target\debug\examples\local_capture.exe'
$reportPath = Join-Path $scriptRoot 'capture-report.mjs'
$runRoot = Join-Path $repoRoot ('target\capture-fixture-' + $Policy + '-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
$stopFile = Join-Path $runRoot 'stop.signal'
$actionsPath = Join-Path $runRoot 'fixture-actions.jsonl'
$stdoutPath = Join-Path $runRoot 'capture.stdout.log'
$stderrPath = Join-Path $runRoot 'capture.stderr.log'
$expectedFactsPath = Join-Path $runRoot 'expected-facts.json'
$expectedFacts = [System.Collections.Generic.List[object]]::new()

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class DystilFixtureNative {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hWnd, uint msg, UIntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extraInfo);
    public const uint LEFTDOWN = 0x0002;
    public const uint LEFTUP = 0x0004;
    public const uint WHEEL = 0x0800;
    public const uint WM_CLOSE = 0x0010;
}
'@

# Edge can host several top-level windows inside the same msedge.exe process.
# Keep the fixture's handle separate from Process.MainWindowHandle, which is
# process-level and may point at an unrelated user tab/window.
$script:fixtureEdgeHandle = [IntPtr]::Zero

function Write-JsonLine([string]$path, [object]$value) {
    Add-Content -LiteralPath $path -Value ($value | ConvertTo-Json -Compress -Depth 8)
}

function Write-Action([string]$name, [string]$status, [object]$details = $null) {
    Write-JsonLine $actionsPath ([ordered]@{
        timestamp = [DateTime]::UtcNow.ToString('o')
        action = $name
        status = $status
        details = $details
    })
}

function Add-Marker([string]$phase, [string]$label) {
    $manifest = Get-Content -Raw -LiteralPath (Join-Path $runRoot 'run.json') | ConvertFrom-Json
    Write-JsonLine (Join-Path $runRoot 'markers.jsonl') ([ordered]@{
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
        notes = 'deterministic local real-app fixture'
    })
}

function Add-ExpectedFact([string]$label, [string]$kind, [string]$text, [string]$evidence = 'frame_text') {
    $trimmed = $text.Trim()
    if ($trimmed.Length -lt 4) { throw "Expected $kind fact for $label is empty" }
    $expectedFacts.Add([ordered]@{ label = $label; kind = $kind; text = $trimmed; evidence = $evidence })
    Write-Action "$label.expected_$kind" 'recorded' @{ text = $trimmed }
}

function Save-ExpectedFacts {
    [ordered]@{ schema_version = 1; facts = @($expectedFacts) } |
        ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $expectedFactsPath
}

function Get-WindowProcess([string]$name, [string]$titlePattern = '') {
    $windows = Get-Process -Name $name -ErrorAction SilentlyContinue |
        Where-Object { $_.MainWindowHandle -ne 0 }
    if ($titlePattern) {
        $windows = $windows | Where-Object { $_.MainWindowTitle -match $titlePattern }
    }
    $windows | Select-Object -First 1
}

function Wait-Window([string]$name, [string]$titlePattern = '', [int]$timeoutSeconds = 20) {
    $deadline = (Get-Date).AddSeconds($timeoutSeconds)
    do {
        $window = Get-WindowProcess $name $titlePattern
        if ($window) { return $window }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    throw "Timed out waiting for $name window"
}

function Wait-ProcessWindow([System.Diagnostics.Process]$process, [int]$timeoutSeconds = 20) {
    $deadline = (Get-Date).AddSeconds($timeoutSeconds)
    do {
        $process.Refresh()
        if ($process.MainWindowHandle -ne 0) { return $process }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    throw "Timed out waiting for process window $($process.ProcessName)"
}

function Get-EdgeTopLevelWindows {
    $edgePids = @(Get-Process -Name 'msedge' -ErrorAction SilentlyContinue | ForEach-Object { [int]$_.Id })
    if ($edgePids.Count -eq 0) { return @() }
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $condition = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Window
    )
    $result = [System.Collections.Generic.List[object]]::new()
    try {
        $windows = $root.FindAll([System.Windows.Automation.TreeScope]::Children, $condition)
        foreach ($window in $windows) {
            try {
                $pid = [int]$window.Current.ProcessId
                $handle = [IntPtr]$window.Current.NativeWindowHandle
                if ($handle -ne [IntPtr]::Zero -and $edgePids -contains $pid) {
                    $result.Add([pscustomobject]@{
                        Handle = $handle
                        ProcessId = $pid
                        Title = $window.Current.Name
                    })
                }
            } catch { }
        }
    } catch { }
    @($result)
}

function Start-FixtureEdge {
    $existingHandles = @(Get-EdgeTopLevelWindows | ForEach-Object { [Int64]$_.Handle })
    Start-Process -FilePath 'msedge.exe' -ArgumentList @('--new-window', 'https://mail.google.com/mail/u/0/#inbox') | Out-Null
    $deadline = (Get-Date).AddSeconds(25)
    do {
        $window = Get-EdgeTopLevelWindows |
            Where-Object { $existingHandles -notcontains [Int64]$_.Handle } |
            Sort-Object @{ Expression = { if ($_.Title -match '(?i)Gmail') { 0 } else { 1 } } } |
            Select-Object -First 1
        if ($window) {
            $script:fixtureEdgeHandle = $window.Handle
            return Get-Process -Id $window.ProcessId
        }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    # Edge may service --new-window through an existing browser process and
    # keep the same top-level handle. Reuse that visible Edge window rather
    # than failing before the fixture can focus it.
    $fallback = Get-Process -Name 'msedge' -ErrorAction SilentlyContinue |
        Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
    if ($fallback) {
        $script:fixtureEdgeHandle = [IntPtr]$fallback.MainWindowHandle
        return $fallback
    }
    throw 'Timed out waiting for the fixture-owned Edge window'
}

function Get-ExplorerWindowHandles {
    $shell = New-Object -ComObject Shell.Application
    @($shell.Windows() | ForEach-Object { [Int64]$_.HWND })
}

function Wait-NewExplorerWindow([Int64[]]$existingHandles, [int]$timeoutSeconds = 15) {
    $deadline = (Get-Date).AddSeconds($timeoutSeconds)
    do {
        $handle = Get-ExplorerWindowHandles | Where-Object { $_ -ne 0 -and $existingHandles -notcontains $_ } | Select-Object -First 1
        if ($null -ne $handle) { return [IntPtr]$handle }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    throw 'Timed out waiting for the fixture-owned Explorer window'
}

function Close-FixtureWindow([IntPtr]$handle, [string]$label) {
    if ($handle -ne [IntPtr]::Zero -and [DystilFixtureNative]::PostMessage($handle, [DystilFixtureNative]::WM_CLOSE, [UIntPtr]::Zero, [IntPtr]::Zero)) {
        Write-Action "cleanup.$label" 'close_requested' @{ handle = $handle.ToInt64() }
    }
}

function Close-FixtureNotepad([IntPtr]$handle) {
    if ($handle -eq [IntPtr]::Zero) { return }
    [void][DystilFixtureNative]::SetForegroundWindow($handle)
    Start-Sleep -Milliseconds 200
    # The document is fixture-owned and never saved. Ctrl+A/Delete removes the
    # generated text before requesting close; `n` only answers Notepad's own
    # discard prompt if that Windows version still considers it modified.
    [System.Windows.Forms.SendKeys]::SendWait('^a{BACKSPACE}')
    [void][DystilFixtureNative]::PostMessage($handle, [DystilFixtureNative]::WM_CLOSE, [UIntPtr]::Zero, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 300
    [System.Windows.Forms.SendKeys]::SendWait('n')
    Write-Action 'cleanup.notepad' 'close_requested' @{ handle = $handle.ToInt64() }
}

function Activate-Window([System.Diagnostics.Process]$window, [string]$label, [int]$observerSettleMs = 1500, [bool]$required = $true) {
    $shell = New-Object -ComObject WScript.Shell
    $foreground = $false
    for ($attempt = 1; $attempt -le 8 -and -not $foreground; $attempt += 1) {
        # Edge can replace its top-level window while navigating. Refresh the
        # process before every focus attempt so a stale MainWindowHandle cannot
        # cause fixture keystrokes to land in Windows Search.
        $window.Refresh()
        $handle = if ($window.ProcessName -eq 'msedge' -and $script:fixtureEdgeHandle -ne [IntPtr]::Zero) {
            $script:fixtureEdgeHandle
        } else {
            [IntPtr]$window.MainWindowHandle
        }
        if ($handle -eq [IntPtr]::Zero) {
            Start-Sleep -Milliseconds 250
            continue
        }
        # Windows permits a foreground request following an Alt transition.
        # This is only used to make the fixture's own window active before its
        # own physical test input; it does not inspect or alter user content.
        [System.Windows.Forms.SendKeys]::SendWait('%')
        [void][DystilFixtureNative]::ShowWindow($handle, 9)
        [void][DystilFixtureNative]::BringWindowToTop($handle)
        [void]$shell.AppActivate($window.Id)
        [void][DystilFixtureNative]::SetForegroundWindow($handle)
        $deadline = (Get-Date).AddSeconds(3)
        do {
            $foregroundHandle = [DystilFixtureNative]::GetForegroundWindow()
            [uint32]$foregroundPid = 0
            [void][DystilFixtureNative]::GetWindowThreadProcessId($foregroundHandle, [ref]$foregroundPid)
            if ($foregroundHandle -eq $handle -or $foregroundPid -eq [uint32]$window.Id) {
                $foreground = $true
                break
            }
            Start-Sleep -Milliseconds 100
        } while ((Get-Date) -lt $deadline)
    }
    if (-not $foreground) {
        if (-not $required) {
            Write-Action $label 'foreground_unconfirmed' @{ process = $window.ProcessName; title = $window.MainWindowTitle }
            return
        }
        throw "Could not foreground $label"
    }
    # Give the recorder's foreground observer time to publish the new context
    # before the fixture emits physical input.
    Start-Sleep -Milliseconds $observerSettleMs
    Write-Action $label 'foregrounded' @{ process = $window.ProcessName; title = $window.MainWindowTitle }
    return
}

function Get-FixtureEdgeHandle([System.Diagnostics.Process]$edge) {
    if ($script:fixtureEdgeHandle -ne [IntPtr]::Zero) {
        return $script:fixtureEdgeHandle
    }
    $edge.Refresh()
    return [IntPtr]$edge.MainWindowHandle
}

function Find-EdgeAddressBar([System.Diagnostics.Process]$edge) {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle((Get-FixtureEdgeHandle $edge))
    $condition = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Edit
    )
    $edits = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition)
    foreach ($edit in $edits) {
        $name = $edit.Current.Name
        if ($name -match '(?i)address.*search|search.*address') {
            return $edit
        }
    }
    throw 'Could not find Edge address bar through UI Automation'
}

function Click-Element([System.Windows.Automation.AutomationElement]$element, [string]$label) {
    $rect = $element.Current.BoundingRectangle
    if ($rect.Width -le 0 -or $rect.Height -le 0) {
        throw "$label has no clickable UIA bounds"
    }
    $x = [int]($rect.Left + ($rect.Width / 2))
    $y = [int]($rect.Top + ($rect.Height / 2))
    [DystilFixtureNative]::SetCursorPos($x, $y) | Out-Null
    [DystilFixtureNative]::mouse_event([DystilFixtureNative]::LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
    [DystilFixtureNative]::mouse_event([DystilFixtureNative]::LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
    Write-Action $label 'clicked' @{ x = $x; y = $y; name = $element.Current.Name; automation_id = $element.Current.AutomationId }
}

function Click-EdgeAddressBarBurst([System.Diagnostics.Process]$edge, [string]$label, [int]$count = 3) {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle((Get-FixtureEdgeHandle $edge))
    $rect = $root.Current.BoundingRectangle
    if ($rect.Width -le 0 -or $rect.Height -le 0) { throw 'Edge window has no visible bounds' }
    $x = [int]($rect.Left + ($rect.Width / 2))
    $y = [int]($rect.Top + 55)
    for ($index = 0; $index -lt $count; $index += 1) {
        [DystilFixtureNative]::SetCursorPos($x, $y) | Out-Null
        [DystilFixtureNative]::mouse_event([DystilFixtureNative]::LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
        [DystilFixtureNative]::mouse_event([DystilFixtureNative]::LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 150
    }
    Write-Action $label 'clicked' @{ x = $x; y = $y; count = $count; target = 'edge_address_bar' }
}

function Scroll-EdgeDocument([System.Diagnostics.Process]$edge, [string]$label, [int]$seconds = 11) {
    Add-Marker 'start' $label
    Activate-Window $edge $label
    $root = [System.Windows.Automation.AutomationElement]::FromHandle((Get-FixtureEdgeHandle $edge))
    $rect = $root.Current.BoundingRectangle
    $x = [int]($rect.Left + ($rect.Width / 2))
    $y = [int]($rect.Top + ($rect.Height * 0.70))
    [DystilFixtureNative]::SetCursorPos($x, $y) | Out-Null
    $until = (Get-Date).AddSeconds($seconds)
    $ticks = 0
    while ((Get-Date) -lt $until) {
        # mouse_event receives a DWORD; preserve the signed -120 wheel delta
        # as its two's-complement bit pattern without PowerShell numeric coercion.
        $wheelDelta = [BitConverter]::ToUInt32([BitConverter]::GetBytes([int]-120), 0)
        [DystilFixtureNative]::mouse_event([DystilFixtureNative]::WHEEL, 0, 0, $wheelDelta, [UIntPtr]::Zero)
        $ticks += 1
        Start-Sleep -Milliseconds 300
    }
    Start-Sleep -Seconds 3
    Write-Action $label 'scrolled' @{ seconds = $seconds; wheel_ticks = $ticks; x = $x; y = $y }
    Add-Marker 'end' $label
}

function Find-EdgeDocumentFact([System.Diagnostics.Process]$edge, [string]$label, [string]$fact) {
    Add-Marker 'start' $label
    Activate-Window $edge $label
    [System.Windows.Forms.SendKeys]::SendWait('^f')
    [System.Windows.Forms.SendKeys]::SendWait($fact)
    Start-Sleep -Milliseconds 700
    [System.Windows.Forms.SendKeys]::SendWait('{ESC}')
    # A physical click makes the currently found document location a
    # deterministic settled-capture demand for both Stage 3 and the candidate.
    # The address bar is inert here: it changes focus but never navigates.
    Click-EdgeAddressBarBurst $edge "$label.capture_demand" 1
    Start-Sleep -Seconds 3
    # The fact is entered into the browser find control. Accept the complete
    # visible frame when UIA exposes the match, or the linked text event when
    # Chromium times out before exposing the document subtree.
    Add-ExpectedFact $label 'document_fact' $fact 'frame_or_linked_event'
    Write-Action $label 'found' @{ text = $fact }
    Add-Marker 'end' $label
}

function Get-VisibleUiTextCandidates([System.Diagnostics.Process]$edge) {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle((Get-FixtureEdgeHandle $edge))
    $walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
    $queue = [System.Collections.Generic.Queue[System.Windows.Automation.AutomationElement]]::new()
    $queue.Enqueue($root)
    $seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    $result = [System.Collections.Generic.List[string]]::new()
    $deadline = (Get-Date).AddMilliseconds(900)
    while ($queue.Count -gt 0 -and $result.Count -lt 80 -and (Get-Date) -lt $deadline) {
        $element = $queue.Dequeue()
        try {
            $name = $element.Current.Name.Trim()
            if ($name.Length -ge 18 -and $name.Length -le 260 -and $name -match '\s+\S+\s+' -and $seen.Add($name)) {
                $result.Add($name)
            }
            $child = $walker.GetFirstChild($element)
            while ($null -ne $child -and $queue.Count -lt 300) {
                $queue.Enqueue($child)
                $child = $walker.GetNextSibling($child)
            }
        } catch {
            # A Gmail virtual row may disappear while this bounded observer is
            # inspecting it. Continue with the remaining visible controls.
        }
    }
    @($result)
}

function Exercise-GmailThreads([System.Diagnostics.Process]$edge) {
    Activate-Window $edge 'gmail_thread_list'
    for ($index = 1; $index -le 3; $index += 1) {
        $label = "gmail_thread_$index"
        Add-Marker 'start' $label
        Activate-Window $edge $label
        # Gmail virtualizes/rebuilds rows after Back; never retain a stale
        # AutomationElement across thread navigations.
        $threads = Get-GmailThreadCandidates $edge
        if ($threads.Count -lt 3) { throw "Could not locate three visible Gmail rows for $label" }
        $thread = $threads[$index - 1]
        Click-Element $thread.Element "$label.mailbox_row"
        Start-Sleep -Seconds 3
        $fact = Compact-GmailFact $thread.Text
        Add-ExpectedFact $label 'gmail_visible_text' $fact
        Write-Action $label 'opened' @{ visible_text = $fact; role = $thread.Role }
        Add-Marker 'end' $label
        [System.Windows.Forms.SendKeys]::SendWait('%{LEFT}')
        Start-Sleep -Seconds 2
    }
}

function Find-GmailDraftBody([System.Diagnostics.Process]$edge) {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle((Get-FixtureEdgeHandle $edge))
    $condition = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Edit
    )
    $best = $null
    $bestArea = 0.0
    foreach ($edit in $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition)) {
        try {
            $name = $edit.Current.Name
            $rect = $edit.Current.BoundingRectangle
            $area = $rect.Width * $rect.Height
            if ($rect.Width -gt 120 -and $rect.Height -gt 40 -and
                $name -notmatch '(?i)search mail|subject|recipients|to|address') {
                if ($area -gt $bestArea) { $best = $edit; $bestArea = $area }
            }
        } catch { }
    }
    if ($null -eq $best) { throw 'Could not find Gmail draft body editor' }
    $best
}

function Exercise-GmailComposeDraft([System.Diagnostics.Process]$edge) {
    Add-Marker 'start' 'gmail_compose_draft'
    Activate-Window $edge 'gmail_compose_draft'
    $compose = Find-VisibleEdgeControl $edge ([System.Windows.Automation.ControlType]::Button) '(?i)^compose$' 3000
    Click-Element $compose 'gmail_compose_draft.compose'
    Start-Sleep -Seconds 2
    $subject = Find-VisibleEdgeControl $edge ([System.Windows.Automation.ControlType]::Edit) '(?i)^subject$' 3000
    $body = Find-GmailDraftBody $edge
    $subjectText = 'Dystil fixture draft subject'
    $bodyText = 'Dystil fixture draft body; do not send'
    Click-Element $subject 'gmail_compose_draft.subject'
    [System.Windows.Forms.SendKeys]::SendWait($subjectText)
    Click-Element $body 'gmail_compose_draft.body'
    [System.Windows.Forms.SendKeys]::SendWait($bodyText)
    Start-Sleep -Seconds 3
    Write-Action 'gmail_compose_draft' 'typed_without_sending' @{ subject = $subjectText; body = $bodyText }
    # Clear the fixture text before closing so this read-only test does not
    # leave user-visible draft content behind. It never clicks Send.
    Click-Element $body 'gmail_compose_draft.clear_body'
    [System.Windows.Forms.SendKeys]::SendWait('^a{BACKSPACE}')
    Click-Element $subject 'gmail_compose_draft.clear_subject'
    [System.Windows.Forms.SendKeys]::SendWait('^a{BACKSPACE}')
    [System.Windows.Forms.SendKeys]::SendWait('{ESC}')
    Start-Sleep -Seconds 1
    Add-Marker 'end' 'gmail_compose_draft'
}

function Find-VisibleEdgeControl([System.Diagnostics.Process]$edge, [System.Windows.Automation.ControlType]$controlType, [string]$namePattern, [int]$timeoutMilliseconds = 1600) {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle((Get-FixtureEdgeHandle $edge))
    $walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
    $queue = [System.Collections.Generic.Queue[System.Windows.Automation.AutomationElement]]::new()
    $queue.Enqueue($root)
    $deadline = (Get-Date).AddMilliseconds($timeoutMilliseconds)
    while ($queue.Count -gt 0 -and (Get-Date) -lt $deadline) {
        $element = $queue.Dequeue()
        try {
            if ($element.Current.ControlType -eq $controlType -and $element.Current.Name -match $namePattern) {
                $rect = $element.Current.BoundingRectangle
                if ($rect.Width -gt 0 -and $rect.Height -gt 0) { return $element }
            }
            $child = $walker.GetFirstChild($element)
            while ($null -ne $child -and $queue.Count -lt 650) {
                $queue.Enqueue($child)
                $child = $walker.GetNextSibling($child)
            }
        } catch {
            # Browser UIA nodes are virtualized while Gmail updates; continue
            # through the bounded queue instead of retaining a stale element.
        }
    }
    throw "Could not find visible $namePattern control in Edge"
}

function Exercise-GmailSearchAndFilter([System.Diagnostics.Process]$edge) {
    Add-Marker 'start' 'gmail_search_filter'
    Activate-Window $edge 'gmail_search_filter'
    $search = Find-VisibleEdgeControl $edge ([System.Windows.Automation.ControlType]::Edit) '(?i)search mail'
    Click-Element $search 'gmail_search_filter.search_field'
    [System.Windows.Forms.SendKeys]::SendWait('^a')
    [System.Windows.Forms.SendKeys]::SendWait('in:inbox')
    [System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
    Start-Sleep -Seconds 3
    Add-ExpectedFact 'gmail_search_filter' 'search_query' 'in:inbox'
    $options = Find-VisibleEdgeControl $edge ([System.Windows.Automation.ControlType]::Button) '(?i)show search options|search options'
    Click-Element $options 'gmail_search_filter.options'
    Start-Sleep -Seconds 3
    # Chromium's capture tree exposes the invoked button but, on this machine,
    # not the popup field labels. Keep the real expansion in fixture-actions
    # and validate the observable query/filter state through the URL and field
    # value instead of pretending that invisible provider data was captured.
    Write-Action 'gmail_search_filter.filter_form' 'expanded' @{ control = 'Advanced search options'; expected_visible_field = 'Has the words' }
    # Close the real Gmail filter form and restore the normal inbox before
    # thread traversal. The opened form remains a captured expansion state.
    [System.Windows.Forms.SendKeys]::SendWait('{ESC}')
    Start-Sleep -Milliseconds 500
    Click-Element $search 'gmail_search_filter.clear_search'
    [System.Windows.Forms.SendKeys]::SendWait('^a')
    [System.Windows.Forms.SendKeys]::SendWait('{BACKSPACE}')
    [System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
    Start-Sleep -Seconds 3
    Write-Action 'gmail_search_filter' 'completed' @{ query = 'in:inbox'; filter_form = 'Show search options' }
    Add-Marker 'end' 'gmail_search_filter'
}

function Compact-GmailFact([string]$text) {
    $clean = ($text -replace '\p{Cf}|\p{Mn}', '') -replace '\s+', ' '
    $parts = @($clean -split '\s*,\s*' | Where-Object { $_.Trim().Length -gt 0 })
    $fact = if ($parts.Count -ge 2) { "$($parts[0]), $($parts[1])" } else { $clean }
    if ($fact.Length -gt 180) { $fact = $fact.Substring(0, 180).TrimEnd() }
    $fact
}

function Get-GmailThreadCandidates([System.Diagnostics.Process]$edge) {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle((Get-FixtureEdgeHandle $edge))
    $rootRect = $root.Current.BoundingRectangle
    $walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
    $queue = [System.Collections.Generic.Queue[System.Windows.Automation.AutomationElement]]::new()
    $queue.Enqueue($root)
    $seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    $result = [System.Collections.Generic.List[object]]::new()
    $deadline = (Get-Date).AddMilliseconds(1200)
    while ($queue.Count -gt 0 -and $result.Count -lt 12 -and (Get-Date) -lt $deadline) {
        $element = $queue.Dequeue()
        try {
            $name = $element.Current.Name.Trim()
            $role = $element.Current.ControlType.ProgrammaticName
            $rect = $element.Current.BoundingRectangle
            $isMailboxRow = $role -match 'DataItem|ListItem' -and
                $rect.Width -gt ($rootRect.Width * 0.35) -and $rect.Height -gt 12 -and
                $rect.Top -gt ($rootRect.Top + 120) -and
                $name.Length -ge 12 -and $name -notmatch 'Microsoft.? Edge|\bPersonal\b'
            if ($isMailboxRow -and $seen.Add($name)) {
                $result.Add([pscustomobject]@{ Element = $element; Text = $name; Role = $role })
            }
            $child = $walker.GetFirstChild($element)
            while ($null -ne $child -and $queue.Count -lt 400) {
                $queue.Enqueue($child)
                $child = $walker.GetNextSibling($child)
            }
        } catch {
            # Gmail virtualizes rows while it paints; retain the stable rows
            # already found and continue through the bounded traversal.
        }
    }
    @($result)
}

function Exercise-Typing([ref]$ownedNotepad) {
    Start-Process -FilePath 'notepad.exe' | Out-Null
    $notepad = Wait-Window 'notepad' '^(Untitled|\*DystilFixtureTyping)' 15
    $ownedNotepad.Value = [IntPtr]$notepad.MainWindowHandle
    Add-Marker 'start' 'typing_pauses'
    Activate-Window $notepad 'typing_pauses'
    $first = 'DystilFixtureTypingStart'
    $last = 'DystilFixtureTypingEnd'
    Add-ExpectedFact 'typing_pauses' 'typing_start' $first 'frame_text'
    Add-ExpectedFact 'typing_pauses' 'typing_end' $last 'frame_text'
    [System.Windows.Forms.SendKeys]::SendWait($first)
    Start-Sleep -Milliseconds 1800
    [System.Windows.Forms.SendKeys]::SendWait(' fixture pause ')
    Start-Sleep -Milliseconds 1800
    [System.Windows.Forms.SendKeys]::SendWait($last)
    Start-Sleep -Seconds 3
    Write-Action 'typing_pauses' 'typed' @{ start = $first; ending = $last; pauses_ms = 1800 }
    Add-Marker 'end' 'typing_pauses'
}

function Exercise-RapidAppSwitch([System.Diagnostics.Process]$edge, [System.Diagnostics.Process]$explorer, [System.Diagnostics.Process]$editor) {
    Add-Marker 'start' 'rapid_app_switch'
    Activate-Window $edge 'rapid_app_switch.edge_a' 400
    Activate-Window $explorer 'rapid_app_switch.explorer_b' 400
    Activate-Window $editor 'rapid_app_switch.editor_c' 400
    Activate-Window $edge 'rapid_app_switch.edge_a_return' 900
    Write-Action 'rapid_app_switch' 'completed' @{ sequence = @('msedge.exe', 'explorer.exe', $editor.ProcessName, 'msedge.exe') }
    Add-Marker 'end' 'rapid_app_switch'
}

function Navigate-Edge([System.Diagnostics.Process]$edge, [string]$url, [string]$label, [int]$settleSeconds = 5) {
    Add-Marker 'start' $label
    Activate-Window $edge $label
    if ($label -eq 'gmail_inbox') { Click-EdgeAddressBarBurst $edge "$label.address_bar" }
    # Do not use FindAll(Descendants) against Gmail/Edge here: it is unbounded
    # and can itself stall the fixture. Ctrl+L is Edge's normal address-bar
    # action and keeps this test focused on Dystil's capture walk.
    [System.Windows.Forms.SendKeys]::SendWait('^l')
    [System.Windows.Forms.SendKeys]::SendWait($url)
    [System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
    Start-Sleep -Seconds $settleSeconds
    Write-Action $label 'navigated' @{ url = $url; title = $edge.MainWindowTitle }
    Add-Marker 'end' $label
}

function Exercise-StaticEdgeSurface([System.Diagnostics.Process]$edge) {
    Add-Marker 'start' 'static_edge_clicks'
    Activate-Window $edge 'static_edge_clicks'
    for ($index = 1; $index -le 3; $index += 1) {
        Click-EdgeAddressBarBurst $edge "static_edge_click_$index" 1
        # This exceeds the click-burst settle delay but remains inside the
        # candidate's same-app cadence window after the first checkpoint.
        Start-Sleep -Milliseconds 800
    }
    Start-Sleep -Seconds 3
    Add-Marker 'end' 'static_edge_clicks'
}

function Exercise-Idle([int]$seconds) {
    # This is intentionally before opening any fixture app.  It measures the
    # recorder with no fixture input at all, rather than merely an unchanged
    # foreground UI after a preceding action.
    Add-Marker 'start' 'genuine_idle'
    Write-Action 'genuine_idle' 'started' @{ seconds = $seconds }
    Start-Sleep -Seconds $seconds
    Write-Action 'genuine_idle' 'completed' @{ seconds = $seconds }
    Add-Marker 'end' 'genuine_idle'
}

function Assert-Run {
    $report = Get-Content -Raw -LiteralPath (Join-Path $runRoot 'comparison.json') | ConvertFrom-Json
    $events = Get-Content -LiteralPath (Join-Path $runRoot 'events.jsonl') | ForEach-Object { $_ | ConvertFrom-Json }
    $physical = @($events | Where-Object { $_.kind -eq 'ui_event' -and $_.event_type -eq 'click' -and $_.source -eq 'physical_click' })
    $edgePhysical = @($physical | Where-Object { $_.app_name -eq 'msedge.exe' })
    $enriched = @($events | Where-Object { $_.kind -eq 'ui_event' -and $_.event_type -eq 'click' -and $_.source -eq 'element_enrichment' -and $_.has_element_context })
    $switches = @($events | Where-Object { $_.kind -eq 'ui_event' -and $_.event_type -eq 'app_switch' })
    $focus = @($events | Where-Object { $_.kind -eq 'ui_event' -and $_.event_type -eq 'window_focus' })
    $scrolls = @($events | Where-Object { $_.kind -eq 'ui_event' -and $_.event_type -eq 'scroll' })
    $keys = @($events | Where-Object { $_.kind -eq 'ui_event' -and $_.event_type -eq 'key' })
    $captures = Get-Content -LiteralPath (Join-Path $runRoot 'captures.jsonl') | ForEach-Object { $_ | ConvertFrom-Json }
    $markers = Get-Content -LiteralPath (Join-Path $runRoot 'markers.jsonl') | ForEach-Object { $_ | ConvertFrom-Json }
    $idleStart = $markers | Where-Object { $_.kind -eq 'scenario_marker' -and $_.marker_id -eq 'genuine_idle' -and $_.phase -eq 'start' } | Select-Object -First 1
    $idleEnd = $markers | Where-Object { $_.kind -eq 'scenario_marker' -and $_.marker_id -eq 'genuine_idle' -and $_.phase -eq 'end' } | Select-Object -First 1
    $idleCaptureRequests = @()
    if ($idleStart -and $idleEnd) {
        $idleStartAt = [DateTimeOffset]::Parse($idleStart.timestamp)
        $idleEndAt = [DateTimeOffset]::Parse($idleEnd.timestamp)
        $idleCaptureRequests = @($captures | Where-Object {
            $_.kind -eq 'capture_request' -and
            ([DateTimeOffset]::Parse($_.timestamp) -ge $idleStartAt) -and
            ([DateTimeOffset]::Parse($_.timestamp) -le $idleEndAt)
        })
    } else {
        $failures += 'genuine idle markers were missing'
    }
    $background = $report.capture.background_by_reason
    $failures = @()
    if ($physical.Count -lt 6) { $failures += 'fewer than six physical clicks observed' }
    if ($edgePhysical.Count -lt 6) { $failures += 'fixture clicks were not attributed to Edge' }
    if ($enriched.Count -lt $physical.Count) { $failures += 'one or more physical clicks lacked element enrichment' }
    if ($report.urls.browser_frames_with_url -lt 1) { $failures += 'no browser frame had a URL' }
    if ($report.expected_facts.unmatched -ne 0) { $failures += 'one or more required visible facts were absent from frame text' }
    $gmailFacts = @($report.expected_facts.matches | Where-Object { $_.kind -eq 'gmail_visible_text' })
    if ($gmailFacts.Count -ne 3 -or @($gmailFacts.expected | Select-Object -Unique).Count -ne 3) {
        $failures += 'three distinct Gmail thread facts were not recorded'
    }
    if (($switches.Count + $focus.Count) -lt 3) { $failures += 'too few desktop transitions observed' }
    if ($scrolls.Count -lt 1) { $failures += 'long-document scrolling was not observed' }
    $scrollSpan = @($report.activity_spans.spans | Where-Object {
        $_.kind -eq 'scroll_burst' -and $_.duration_ms -ge 2500 -and $_.scroll_delta_y -ne 0 -and $null -ne $_.final_frame_id
    })
    if ($scrollSpan.Count -lt 1) { $failures += 'long-document scroll span was not linked to a final frame' }
    if ($keys.Count -lt 10) { $failures += 'typing activity was not observed' }
    if ($idleCaptureRequests.Count -ne 0) { $failures += "genuine idle produced $($idleCaptureRequests.Count) capture request(s)" }
    if ($Policy -eq 'stage4_visible_relevant') {
        if ($report.events.stored_rows -ne $report.events.linked_rows) {
            $failures += 'candidate left one or more stored UI events without a frame link'
        }
        if ($report.activity_spans.count -ne $report.activity_spans.linked_final_frames) {
            $failures += 'candidate left one or more activity spans without a final frame link'
        }
    }
    $focusTrees = if ($null -eq $background.focus) { 0 } else { [int]$background.focus }
    $periodicTrees = if ($null -eq $background.periodic) { 0 } else { [int]$background.periodic }
    if ($Policy -in @('stage1_no_background_trees', 'stage3_settled_state', 'stage4_visible_relevant') -and ($focusTrees -ne 0 -or $periodicTrees -ne 0)) {
        $failures += 'candidate performed focus or periodic background tree work'
    }
    $validation = [ordered]@{
        policy = $Policy
        passed = $failures.Count -eq 0
        failures = $failures
        physical_clicks = $physical.Count
        edge_physical_clicks = $edgePhysical.Count
        enriched_clicks = $enriched.Count
        browser_frames_with_url = $report.urls.browser_frames_with_url
        browser_frames = $report.urls.browser_frames
        app_switches = $switches.Count
        window_focuses = $focus.Count
        scroll_events = $scrolls.Count
        key_events = $keys.Count
        idle_capture_requests = $idleCaptureRequests.Count
        gmail_visible_facts = $gmailFacts.Count
        background_by_reason = $background
    }
    $validation | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $runRoot 'fixture-validation.json')
    if ($failures.Count -gt 0) { throw ('Fixture validation failed: ' + ($failures -join '; ')) }
}

if (-not (Test-Path -LiteralPath $binaryPath)) {
    throw "Capture binary missing: $binaryPath. Build it with cargo build -p dystil-capture --example local_capture --features native,debug-capture"
}

if ($DryRun) {
    Write-Host 'Dry run passed.'
    Write-Host 'Fixture actions: real Edge address-bar navigation, read-only Gmail/public-web traversal, and Explorer plus editor foregrounding.'
    Write-Host 'No browser, desktop app, or capture process was started.'
    exit 0
}

New-Item -ItemType Directory -Path $runRoot | Out-Null
New-Item -ItemType File -Path $actionsPath | Out-Null
$capture = $null
$fixtureEdgeHandle = [IntPtr]::Zero
$fixtureExplorerHandle = [IntPtr]::Zero
$fixtureNotepadHandle = [IntPtr]::Zero
try {
    $capture = Start-Process -FilePath $binaryPath -ArgumentList @('--text-only', '--capture-scroll', '--diagnostics', '--policy', $Policy, '--measurement-mode', $MeasurementMode, '--stop-file', $stopFile, '--data-dir', $runRoot) -WindowStyle Hidden -PassThru -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
    for ($attempt = 0; $attempt -lt 80 -and -not (Test-Path -LiteralPath (Join-Path $runRoot 'run.json')); $attempt += 1) { Start-Sleep -Milliseconds 250 }
    if (-not (Test-Path -LiteralPath (Join-Path $runRoot 'run.json'))) { throw 'Capture harness did not initialize.' }

    # Starting a hidden recorder can itself cause one foreground transition as
    # the invoking shell yields focus.  Let that startup transition settle
    # before beginning the genuinely-idle measurement window.
    Start-Sleep -Seconds 3
    # Use a blank fixture-owned Notepad for idle: browser pages can emit
    # accessibility/typing-like notifications while merely loading mail.
    Start-Process -FilePath 'notepad.exe' | Out-Null
    $idleNotepad = Wait-Window 'notepad' '^(Untitled|\*DystilFixtureTyping)' 15
    $idleNotepadHandle = [IntPtr]$idleNotepad.MainWindowHandle
    # Idle needs no injected input. If Windows focus-stealing protection denies
    # Notepad, continue the idle interval rather than turning a harmless setup
    # limitation into a false capture failure.
    Activate-Window $idleNotepad 'genuine_idle_setup' 1500 $false | Out-Null
    Start-Sleep -Seconds 2
    Exercise-Idle $IdleSeconds
    Close-FixtureWindow $idleNotepadHandle 'idle_notepad'

    $edge = Start-FixtureEdge
    $fixtureEdgeHandle = Get-FixtureEdgeHandle $edge
    Navigate-Edge $edge 'https://mail.google.com/mail/u/0/#inbox' 'gmail_inbox' 7
    Exercise-StaticEdgeSurface $edge
    Exercise-GmailSearchAndFilter $edge
    Navigate-Edge $edge 'https://example.com/' 'public_web_traversal' 4
    # Sealed generic browser holdout: this site is not used to define any
    # classifier. Read-only navigation verifies the generic fallback remains
    # legible on an unseen public surface.
    Navigate-Edge $edge 'https://www.wikipedia.org/' 'unknown_public_holdout' 5
    Find-EdgeDocumentFact $edge 'unknown_public_holdout_fact' 'Wikipedia'
    Navigate-Edge $edge 'https://www.rfc-editor.org/rfc/rfc9110' 'long_http_document' 5
    Find-EdgeDocumentFact $edge 'long_http_document_start' 'HTTP Semantics'
    Find-EdgeDocumentFact $edge 'long_http_document_middle' 'request method'
    Find-EdgeDocumentFact $edge 'long_http_document_end' 'Status Codes'
    Scroll-EdgeDocument $edge 'long_http_document_scroll' 11
    Navigate-Edge $edge 'https://mail.google.com/mail/u/0/#inbox' 'gmail_return' 5
    Exercise-GmailThreads $edge
    Exercise-GmailComposeDraft $edge

    $existingExplorerHandles = Get-ExplorerWindowHandles
    Start-Process -FilePath 'explorer.exe' -ArgumentList $repoRoot | Out-Null
    $fixtureExplorerHandle = Wait-NewExplorerWindow $existingExplorerHandles
    $explorer = Wait-Window 'explorer' '' 15
    Add-Marker 'start' 'file_explorer'
    Activate-Window $explorer 'file_explorer'
    [System.Windows.Forms.SendKeys]::SendWait('^l')
    [System.Windows.Forms.SendKeys]::SendWait($repoRoot)
    [System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
    Start-Sleep -Seconds 2
    Add-Marker 'end' 'file_explorer'

    $code = Get-WindowProcess 'Code'
    $editorLabel = 'vs_code'
    if (-not $code) {
        Start-Process -FilePath 'notepad.exe' | Out-Null
        $code = Wait-Window 'notepad' '' 20
        $editorLabel = 'notepad_fallback'
    }
    Add-Marker 'start' $editorLabel
    Activate-Window $code $editorLabel
    if ($code.ProcessName -eq 'Code') {
        [System.Windows.Forms.SendKeys]::SendWait('^p')
        [System.Windows.Forms.SendKeys]::SendWait('CAPTURE_IMPROVEMENT_PLAN.md')
        [System.Windows.Forms.SendKeys]::SendWait('{ESC}')
    } else {
        [System.Windows.Forms.SendKeys]::SendWait('DystilFixtureSearchNeedle')
    }
    Start-Sleep -Seconds 3
    Add-Marker 'end' $editorLabel

    Exercise-RapidAppSwitch $edge $explorer $code
    Exercise-Typing ([ref]$fixtureNotepadHandle)
    # Let the recorder's final typing-pause trigger finish linking before the
    # stop signal. Without this drain, the last text event can be flushed by
    # the native hook after the capture consumer has already shut down.
    Start-Sleep -Seconds 3
    Save-ExpectedFacts

    New-Item -ItemType File -Path $stopFile | Out-Null
    $capture.WaitForExit(15000)
    if (-not $capture.HasExited) { throw 'Capture harness did not stop after the fixture.' }
    & bun $reportPath analyze --run-dir $runRoot
    if ($LASTEXITCODE -ne 0) { throw 'Report generation failed.' }
    Assert-Run
    Write-Host "Fixture passed. Report: $(Join-Path $runRoot 'comparison.md')" -ForegroundColor Green
    if ($OpenReport) { Start-Process (Join-Path $runRoot 'comparison.md') }
} catch {
    Write-Action 'fixture' 'failed' @{ message = $_.Exception.Message }
    throw
} finally {
    if ($capture -and -not $capture.HasExited) {
        New-Item -ItemType File -Path $stopFile -Force | Out-Null
    }
    if (-not $KeepFixtureApps) {
        Close-FixtureNotepad $fixtureNotepadHandle
        Close-FixtureWindow $fixtureExplorerHandle 'explorer'
        Close-FixtureWindow $fixtureEdgeHandle 'edge'
    }
}
