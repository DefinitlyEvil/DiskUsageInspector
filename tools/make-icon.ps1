# Generates res/app.ico: a donut/usage-chart mark on a dark circular badge,
# emitted at several sizes as PNG-compressed ICO entries (Vista+ format).
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$root = Split-Path -Parent $PSScriptRoot
$sizes = 16, 24, 32, 48, 64, 128, 256

$bg = [System.Drawing.Color]::FromArgb(255, 26, 27, 32)
$segments = @(
    @{ color = [System.Drawing.Color]::FromArgb(255, 76, 141, 255); sweep = 162.0 },  # 45% blue
    @{ color = [System.Drawing.Color]::FromArgb(255, 63, 185, 80);  sweep = 90.0 },   # 25% green
    @{ color = [System.Drawing.Color]::FromArgb(255, 210, 153, 34); sweep = 64.8 },   # 18% amber
    @{ color = [System.Drawing.Color]::FromArgb(255, 248, 81, 73);  sweep = 43.2 }    # 12% red
)

function New-IconPng([int]$s) {
    $bmp = New-Object System.Drawing.Bitmap($s, $s, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.Clear([System.Drawing.Color]::Transparent)

    $bgBrush = New-Object System.Drawing.SolidBrush($bg)
    $g.FillEllipse($bgBrush, 0, 0, ($s - 1), ($s - 1))

    $inset = [single]([math]::Round($s * 0.14))
    $d = [single]($s - 2 * $inset)
    $angle = [single](-90.0)
    foreach ($seg in $segments) {
        $brush = New-Object System.Drawing.SolidBrush($seg.color)
        $g.FillPie($brush, $inset, $inset, $d, $d, $angle, [single]($seg.sweep - 3.0))
        $brush.Dispose()
        $angle += [single]$seg.sweep
    }

    $hub = [single]([math]::Round($s * 0.30))
    $hubOffset = [single]([math]::Round(($s - $hub) / 2))
    $g.FillEllipse($bgBrush, $hubOffset, $hubOffset, $hub, $hub)
    $bgBrush.Dispose()
    $g.Dispose()

    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    $bytes = $ms.ToArray()
    $ms.Dispose()
    return , [byte[]]$bytes
}

$payloads = New-Object System.Collections.Generic.List[object]
foreach ($s in $sizes) {
    $bytes = [byte[]](New-IconPng $s)
    if ($bytes.Length -lt 100) { throw "icon payload for ${s}px too small: $($bytes.Length) bytes" }
    $payloads.Add([pscustomobject]@{ size = $s; bytes = $bytes })
    "  ${s}px -> $($bytes.Length) bytes"
}

$out = Join-Path $root 'res\app.ico'
New-Item -ItemType Directory -Force -Path (Split-Path $out) | Out-Null
$fs = [System.IO.File]::Create($out)
$bw = New-Object System.IO.BinaryWriter($fs)
$bw.Write([UInt16]0)                 # reserved
$bw.Write([UInt16]1)                 # type: icon
$bw.Write([UInt16]$payloads.Count)   # image count
$offset = 6 + 16 * $payloads.Count
foreach ($p in $payloads) {
    $dim = if ($p.size -ge 256) { 0 } else { $p.size }
    $bw.Write([Byte]$dim)
    $bw.Write([Byte]$dim)
    $bw.Write([Byte]0)
    $bw.Write([Byte]0)
    $bw.Write([UInt16]1)
    $bw.Write([UInt16]32)
    $bw.Write([UInt32]$p.bytes.Length)
    $bw.Write([UInt32]$offset)
    $offset += $p.bytes.Length
}
foreach ($p in $payloads) { $bw.Write([byte[]]$p.bytes) }
$bw.Flush(); $bw.Dispose(); $fs.Dispose()

$ico = Get-Item $out
"wrote $out ($($ico.Length) bytes, $($payloads.Count) sizes)"

foreach ($want in 256, 48, 16) {
    $p = $payloads | Where-Object { $_.size -eq $want }
    [System.IO.File]::WriteAllBytes((Join-Path $env:TEMP "app-icon-$want.png"), [byte[]]$p.bytes)
}
"previews: $env:TEMP\app-icon-{256,48,16}.png"
