# Compara o mesmo pedido pelo hangar-server (Rust, porta 8765) e direto no Python (porta interna),
# para cada parte ja migrada. So leitura: nao muda nada no servidor. Parte que o Rust ainda nao
# atende ele repassa ao Python, e ai os dois tempos ficam parecidos.
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
$rust = '127.0.0.1:8765'
$py = $match.Matches[0].Groups[1].Value

# curl.exe, nao Invoke-WebRequest: o custo fixo do iwr no 5.1 (~30 ms) esconde a diferenca.
function Request-Ms([string]$hostPort, [string]$path) {
    $url = "http://$hostPort$path"
    $out = curl.exe -s -m 30 -o NUL -w '%{http_code} %{time_total}' -H "Authorization: Bearer $token" $url
    $code, $secs = $out -split ' '
    if ($LASTEXITCODE -ne 0 -or $code -ne '200') { throw "erro HTTP $code (curl $LASTEXITCODE) em $url" }
    [double]$secs * 1000
}

# Ate a primeira mensagem do chat ao vivo; o ping inicial nao conta.
function FirstMessage-Ms([string]$hostPort, [string]$name) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $req = [Net.HttpWebRequest]::Create("http://$hostPort/api/sessions/$name/events")
    $req.Headers.Add('Authorization', "Bearer $token")
    $req.Timeout = 3000
    $req.ReadWriteTimeout = 3000
    $resp = $req.GetResponse()
    try {
        $reader = New-Object IO.StreamReader($resp.GetResponseStream())
        while ($true) {
            $line = $reader.ReadLine()
            if ($null -eq $line) { throw "canal fechou sem mensagem em $name" }
            if ($line -eq 'event: message') { return $watch.Elapsed.TotalMilliseconds }
        }
    } finally { $resp.Close() }
}

# Rust e Python alternados: 1 aquecimento + 5 medidas.
function Pair($fn, [string]$arg) {
    $r = @(); $p = @()
    foreach ($i in 0..5) {
        $a = & $fn $rust $arg
        $b = & $fn $py $arg
        if ($i -gt 0) { $r += $a; $p += $b }
    }
    @([math]::Round(($r | Measure-Object -Average).Average), [math]::Round(($p | Measure-Object -Average).Average))
}

function Row([string]$label, [string]$kind, $pair) {
    $ganho = if ($pair[0] -gt 0) { '{0:N1}x' -f ($pair[1] / $pair[0]) } else { '-' }
    '{0,-34} {1,-16} {2,6} ms {3,6} ms {4,7}' -f $label, $kind, $pair[0], $pair[1], $ganho
}

# Parenteses: no PowerShell 5.1 a lista JSON chega ao pipeline como um objeto so.
$sessions = (Invoke-RestMethod -UseBasicParsing -Headers $headers "http://$rust/api/sessions") |
    Where-Object { $_.jsonl }
if (-not $sessions) { Write-Host 'nenhuma sessao aberta com conversa'; exit 1 }

'{0,-34} {1,-16} {2,9} {3,9} {4,7}' -f 'medida', 'tipo', 'Rust', 'Python', 'ganho'
'- Abrir o chat (ultimas 200 mensagens, parte 1)'
foreach ($s in $sessions) { Row $s.name $s.provider (Pair ${function:Request-Ms} "/api/sessions/$($s.name)/history?limit=200") }
'- Chat ao vivo: ate a primeira mensagem (parte 1 e 2B)'
foreach ($s in $sessions) {
    try { Row $s.name $s.provider (Pair ${function:FirstMessage-Ms} $s.name) } catch { Write-Host "$($s.name): $_" }
}
# Com o Rust de pe o Python nao roda Git/arquivos: a rota dele chama o nucleo Rust pela ponte,
# entao a diferenca aqui e o custo da ponte, nao Python contra Rust.
$first = $sessions[0].name
"- Git e arquivos do painel da sessao (PR #30), na sessao $first"
foreach ($rota in @('branches', 'git/files', 'git/log?n=50', 'files/list?so_modificados=false',
                    'files/read?path=README.md', 'files/search?q=import&mode=names')) {
    $label = ($rota -split '\?')[0]
    try { Row $label 'git' (Pair ${function:Request-Ms} "/api/sessions/$first/$rota") } catch { Write-Host "${label}: nao respondeu 200, pulado" }
}
Row 'fs/roots' 'arquivos' (Pair ${function:Request-Ms} '/api/fs/roots')

'- Lista de worktrees (Rust desde feat/worktrees-rust; o Python ainda tem a rota antiga)'
try { Row 'worktrees' 'lista' (Pair ${function:Request-Ms} '/api/worktrees') } catch { Write-Host 'worktrees: nao respondeu 200, pulado' }

# Rust serve a lista publicada pelo ListHub; o Python direto ainda varre processos e psmux e
# classifica cada pane, como fazia antes da troca.
'- Lista de sessoes do dono (Rust desde feat/session-list-state)'
try { Row 'lista de sessoes' 'lista' (Pair ${function:Request-Ms} '/api/sessions') } catch { Write-Host 'lista de sessoes: nao respondeu 200, pulado' }

# O lado que nao e dono responde 202 ate montar o proprio indice.
function Warm([string]$path) {
    foreach ($h in @($rust, $py)) {
        $code = ''
        foreach ($i in 1..120) {
            $code = curl.exe -s -o NUL -w '%{http_code}' -H "Authorization: Bearer $token" "http://$h$path"
            if ($code -eq '200') { break }
            Start-Sleep -Seconds 1
        }
        if ($code -ne '200') { Write-Host "$path em $h ainda respondia $code depois de 120 s" }
    }
}
# A tela inicial pede ?view=summary; o Python ignora o parametro e manda o relatorio inteiro, que
# era o que a tela recebia antes. Por isso a coluna Python desta linha e o "antes".
'- Custos e Uso (parte 3)'
Warm '/api/costs?period=all'; Warm '/api/uso'
Row 'custos: tela inicial' 'resumo' (Pair ${function:Request-Ms} '/api/costs?period=all&view=summary')
Row 'custos: tela Custos' 'inteiro' (Pair ${function:Request-Ms} '/api/costs?period=all')
Row 'uso' '-' (Pair ${function:Request-Ms} '/api/uso')
foreach ($alvo in @('/api/costs?period=all&view=summary', '/api/costs?period=all')) {
    $r = curl.exe -s -H "Authorization: Bearer $token" -H 'Accept-Encoding: gzip' -o NUL -w '%{size_download}' "http://$rust$alvo"
    $p = curl.exe -s -H "Authorization: Bearer $token" -H 'Accept-Encoding: gzip' -o NUL -w '%{size_download}' "http://$py$alvo"
    '{0,-34} {1,-16} {2,6} KB {3,6} KB  (baixado, comprimido)' -f $alvo.Substring(5), 'tamanho', [math]::Floor([double]$r / 1024), [math]::Floor([double]$p / 1024)
}
''
'Media de 5 medidas depois de 1 de aquecimento, Rust e Python alternados. Menos e melhor.'
'Ganho perto de 1x = o Rust ainda repassa essa parte ao Python.'
'Fora da medicao: envio de mensagem, fila e controle das sessoes, as acoes de Git e de worktree'
'que escrevem e a reconstrucao do indice de custos.'
