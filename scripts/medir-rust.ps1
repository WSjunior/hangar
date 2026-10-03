# Compara, nas sessoes abertas, o tempo de abrir o chat pelo hangar-server (Rust, porta 8765) e
# pelo Python (porta interna). So leitura: nao muda nada no servidor.
# Uso: powershell -ExecutionPolicy Bypass -File scripts\medir-rust.ps1
$ErrorActionPreference = 'Stop'

$proc = Get-CimInstance Win32_Process |
    Where-Object { $_.CommandLine -like '*-m app.main*' -and $_.Name -like 'python*' } |
    Select-Object -First 1
if (-not $proc) { Write-Host 'backend do Hangar nao esta rodando'; exit 1 }
# python.exe fica em backend\.venv\Scripts; o .env fica em backend.
$backend = Split-Path (Split-Path (Split-Path $proc.ExecutablePath))
$tokenLine = Get-Content (Join-Path $backend '.env') | Where-Object { $_ -like 'CP_AUTH_TOKEN=*' } | Select-Object -First 1
$token = $tokenLine.Substring('CP_AUTH_TOKEN='.Length)
$headers = @{ Authorization = "Bearer $token" }

try {
    Invoke-RestMethod -UseBasicParsing 'http://127.0.0.1:8765/__hangar_server/health' | Out-Null
} catch {
    Write-Host 'o Rust nao esta atendendo a porta 8765 (o Python esta sozinho)'; exit 1
}
$logRoot = if ($env:LOCALAPPDATA) { $env:LOCALAPPDATA } else { Join-Path $HOME 'AppData\Local' }
$log = Join-Path $logRoot 'hangar\logs\privado\hangar-server.log'
$match = Select-String -Path $log -Pattern 'upstream=(127\.0\.0\.1:\d+)' -ErrorAction SilentlyContinue | Select-Object -Last 1
if (-not $match) { Write-Host "porta do Python nao encontrada em $log"; exit 1 }
$pythonHost = $match.Matches[0].Groups[1].Value

function Media-Ms([string]$hostPort, [string]$name) {
    $url = "http://$hostPort/api/sessions/$name/history?limit=200"
    $times = foreach ($i in 1..6) {
        (Measure-Command { Invoke-WebRequest -UseBasicParsing -Headers $headers $url | Out-Null }).TotalMilliseconds
    }
    # A primeira leitura e o aquecimento.
    [math]::Round((($times | Select-Object -Skip 1) | Measure-Object -Average).Average)
}

# Parenteses: no PowerShell 5.1 a lista JSON chega ao pipeline como um objeto so.
$sessions = (Invoke-RestMethod -UseBasicParsing -Headers $headers 'http://127.0.0.1:8765/api/sessions') |
    Where-Object { $_.jsonl }
if (-not $sessions) { Write-Host 'nenhuma sessao aberta com conversa'; exit 1 }

'{0,-28} {1,-16} {2,9} {3,9} {4,7}' -f 'sessao', 'tipo', 'Rust', 'Python', 'ganho'
foreach ($s in $sessions) {
    $r = Media-Ms '127.0.0.1:8765' $s.name
    $p = Media-Ms $pythonHost $s.name
    $ganho = if ($r -gt 0) { '{0:N1}x' -f ($p / $r) } else { '-' }
    '{0,-28} {1,-16} {2,6} ms {3,6} ms {4,7}' -f $s.name, $s.provider, $r, $p, $ganho
}
''
'Media de 5 aberturas (ultimas 200 mensagens) depois de 1 de aquecimento. Menos e melhor.'
'Pi, Kimi e omp ainda passam pelo Python: nelas os dois tempos ficam parecidos.'
