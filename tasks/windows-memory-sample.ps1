# Read-only native Windows sampler. Start the 0.4.0 TUI first, then run:
#   .\tasks\windows-memory-sample.ps1 -Scenario idle > .\toki-memory-idle.csv
# Repeat with -Scenario slow-save or running-timer. No credentials or notes are recorded.
# The script stops sampling early; CLOSE THE TUI YOURSELF if it warns about growth.
param(
    [ValidateSet('idle', 'running-timer', 'slow-save', 'other')]
    [string]$Scenario = 'idle',
    [int]$Seconds = 900,
    [int]$IntervalSeconds = 10
)

$names = @('toki-tui', 'WindowsTerminal', 'OpenConsole', 'conhost', 'wslhost')
$baseline = @{}
$gib = 1GB
$start = Get-Date
'utc,scenario,name,pid,private_bytes,working_set_bytes,handles,cpu_seconds'

while (((Get-Date) - $start).TotalSeconds -lt $Seconds) {
    $seenTui = $false
    $stop = $false
    $processes = @(Get-Process -Name $names -ErrorAction SilentlyContinue)
    foreach ($p in $processes) {
        if ($p.ProcessName -eq 'toki-tui') { $seenTui = $true }
        try {
            $key = "{0}:{1}" -f $p.ProcessName, $p.Id
            $private = [long]$p.PrivateMemorySize64
            if (-not $baseline.ContainsKey($key)) { $baseline[$key] = $private }
            $stamp = (Get-Date).ToUniversalTime().ToString('o')
            '{0},{1},{2},{3},{4},{5},{6},{7}' -f $stamp, $Scenario, $p.ProcessName,
                $p.Id, $private, $p.WorkingSet64, $p.Handles, $p.CPU.TotalSeconds

            if ($private -ge 2 * $gib -or $private - $baseline[$key] -ge $gib) {
                Write-Warning "Safety limit reached for $($p.ProcessName) PID $($p.Id). Stop the TUI manually; do not let it reach multi-GB usage."
                $stop = $true
            }
        } catch {
            # Process exited between enumeration and sampling.
        }
    }
    if (-not $seenTui) {
        Write-Warning 'No native toki-tui.exe found. If using WSL, this only measures Windows host processes; capture Linux RSS separately.'
        break
    }
    if ($stop) { break }
    Start-Sleep -Seconds $IntervalSeconds
}
