# Which LibreOffice versions the mirrors list, and whether each download exists (lo-check.log).
Set-Location (Split-Path $PSScriptRoot -Parent)
$mirrors = @("https://download.documentfoundation.org/libreoffice/stable/", "https://ftp.fau.de/tdf/libreoffice/stable/", "https://mirror.netcologne.de/tdf/libreoffice/stable/")
$out = @()
$page = (& curl.exe -sL $mirrors[0]) -join " "
$versions = [regex]::Matches($page, 'href="(\d+\.\d+\.\d+)/"') | ForEach-Object { $_.Groups[1].Value } | Sort-Object { [version]$_ } -Descending | Select-Object -Unique
$out += "listed: " + ($versions -join ", ")
foreach ($ver in ($versions | Select-Object -First 3)) {
  foreach ($b in $mirrors) {
    foreach ($p in @("$ver/win/x86_64/LibreOffice_${ver}_Win_x86-64.msi", "$ver/mac/aarch64/LibreOffice_${ver}_MacOS_aarch64.dmg")) {
      $r = & curl.exe -sIL -o NUL -w "%{http_code} %{url_effective}" ($b + $p)
      $out += "$r   <=  $b$p"
    }
  }
}
$out | Set-Content lo-check.log -Encoding utf8
