# Read-only native Windows sampler. Start the 0.4.0 release's
# toki-tui-windows.exe (or a locally renamed toki-tui.exe) first, then run:
#   .\tasks\windows-memory-sample.ps1 -Scenario idle > .\toki-memory-idle.csv
# Repeat with -Scenario slow-save or running-timer. No credentials or notes are recorded.
# The script stops sampling early; CLOSE THE TUI YOURSELF if it warns about growth.
param(
    [ValidateSet('idle', 'running-timer', 'slow-save', 'other')]
    [string]$Scenario = 'idle',
    [int]$Seconds = 900,
    [int]$IntervalSeconds = 10
)

$tuiNames = @('toki-tui', 'toki-tui-windows')
$names = $tuiNames + @('WindowsTerminal', 'OpenConsole', 'conhost', 'wslhost')
$baseline = @{}
$gib = 1GB
$start = Get-Date
'utc,scenario,name,pid,private_bytes,working_set_bytes,handles,cpu_seconds'

while (((Get-Date) - $start).TotalSeconds -lt $Seconds) {
    $seenTui = $false
    $stop = $false
    $processes = @(Get-Process -Name $names -ErrorAction SilentlyContinue)
    foreach ($p in $processes) {
        if ($p.ProcessName -in $tuiNames) { $seenTui = $true }
        try {
            $key = "{0}:{1}" -f $p.ProcessName, $p.Id
            $private = [long]$p.PrivateMemorySize64
            if (-not $baseline.ContainsKey($key)) { $baseline[$key] = $private }
            $stamp = (Get-Date).ToUniversalTime().ToString('o')
            $cpuSeconds = if ($null -ne $p.CPU) {
                ([double]$p.CPU).ToString('0.###', [Globalization.CultureInfo]::InvariantCulture)
            } else { '' }
            '{0},{1},{2},{3},{4},{5},{6},{7}' -f $stamp, $Scenario, $p.ProcessName,
                $p.Id, $private, $p.WorkingSet64, $p.Handles, $cpuSeconds

            if ($private -ge 2 * $gib) {
                Write-Warning "Absolute safety limit reached for $($p.ProcessName) PID $($p.Id) (may predate this sample; no growth proven). Close the test TUI manually."
                $stop = $true
            } elseif ($private - $baseline[$key] -ge $gib) {
                Write-Warning "Private memory grew by at least 1 GiB for $($p.ProcessName) PID $($p.Id). Close the test TUI manually."
                $stop = $true
            }
        } catch {
            # Process exited between enumeration and sampling.
        }
    }
    if (-not $seenTui) {
        Write-Warning 'No native toki-tui.exe or toki-tui-windows.exe found. WSL processes need separate Linux RSS measurements.'
        break
    }
    if ($stop) { break }
    Start-Sleep -Seconds $IntervalSeconds
}
