# Runs a command and reports its wall time and peak working set (Windows), for the prep benchmarks.
#
#   powershell -File scripts/peak.ps1 target\release\kawarimi-prep.exe ".cache\clips\cuts.mp4 --out .cache\prep\x"
#
# The program's arguments go as one string, so PowerShell does not read them as its own.
param([Parameter(Mandatory)][string]$Exe, [string]$Arguments = "")
$watch = [Diagnostics.Stopwatch]::StartNew()
$log = [IO.Path]::GetTempFileName()
$p = Start-Process -FilePath $Exe -ArgumentList $Arguments -NoNewWindow -PassThru -RedirectStandardError $log
$peak = 0
while (-not $p.HasExited) {
    try { $p.Refresh(); if ($p.PeakWorkingSet64 -gt $peak) { $peak = $p.PeakWorkingSet64 } } catch {}
    Start-Sleep -Milliseconds 100
}
$p.WaitForExit()
Get-Content $log
Remove-Item $log -ErrorAction SilentlyContinue
"exit $($p.ExitCode), wall $([math]::Round($watch.Elapsed.TotalSeconds, 1)) s, peak working set $([math]::Round($peak / 1MB)) MB"
