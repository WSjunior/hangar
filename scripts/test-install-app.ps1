# Testa as decisoes do modo -App do install.ps1 sem instalar nada: extrai as funcoes pelo AST e
# roda cada uma com dubles. Uso (Windows PowerShell 5.1):
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\test-install-app.ps1
$ErrorActionPreference = 'Stop'
$installer = Join-Path (Split-Path -Parent $PSScriptRoot) 'install.ps1'
$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($installer, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw ($parseErrors | Out-String) }

$script:falhas = 0
function Assert($condition, $message) {
    if ($condition) { [Console]::WriteLine("ok   $message") } else { [Console]::WriteLine("FAIL $message"); $script:falhas++ }
}
# Dubles: Write-Host vira lista (as marcas sao conferidas por linha) e nada pode esperar teclado.
$script:saida = New-Object 'System.Collections.Generic.List[string]'
function Write-Host { param([Parameter(Position = 0)]$Object, $ForegroundColor, [switch]$NoNewline) $script:saida.Add("$Object") }
function Read-Host { throw 'Read-Host chamado: o -App nao pode esperar teclado' }
function Limpa { $script:saida.Clear() }

foreach ($name in @('Nota', 'Ok', 'Falta', 'Erro', 'Pergunte-Mesmo', 'Pergunte', 'Eleva-E-Roda', 'Mark-Step')) {
    $def = $ast.Find({ param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq $name }, $true)
    if (-not $def) { throw "Funcao ausente: $name" }
    . ([scriptblock]::Create($def.Extent.Text))
}

# Parametros novos
$params = @{}
foreach ($p in $ast.ParamBlock.Parameters) { $params[$p.Name.VariablePath.UserPath] = $p }
Assert ($params.ContainsKey('App') -and $params['App'].StaticType -eq [switch]) 'install.ps1 aceita -App (switch)'
Assert ($params.ContainsKey('SemNativo') -and $params['SemNativo'].StaticType -eq [switch]) 'install.ps1 aceita -SemNativo (switch)'
$vs = @($params['Tailscale'].Attributes | Where-Object { $_.TypeName.Name -eq 'ValidateSet' })
Assert ($vs.Count -eq 1 -and (($vs[0].PositionalArguments | ForEach-Object { $_.Value }) -join ',') -eq 'sim,nao') '-Tailscale so aceita sim ou nao'

# Respostas pela tabela
$App = $true; $Sim = $false; $Avancado = $false; $script:Interativo = $false
Assert ((Pergunte-Mesmo 'Ligar o Modo Desenvolvedor?') -eq $true) '-App: Pergunte-Mesmo responde sim sem ler teclado'
Assert ((Pergunte 'Instalar (recomendado)?') -eq $true) '-App: Pergunte responde sim'
$App = $false
Assert ((Pergunte-Mesmo 'Ligar?') -eq $false) 'sem -App e sem terminal: Pergunte-Mesmo continua nao'
Assert ((Pergunte 'Instalar?') -eq $false) 'sem -App e sem terminal: Pergunte continua nao'

# UAC sem terminal
function EhAdmin { return $false }
$script:startCalls = 0
function Start-Process { param($FilePath, $Verb, [switch]$Wait, [switch]$PassThru, $ArgumentList) $script:startCalls++; return [pscustomobject]@{ ExitCode = 0 } }
$PowerShellExe = 'powershell.exe'
$App = $true; $script:Interativo = $false
Assert ((Eleva-E-Roda 'liberar a porta' 'exit 0') -eq $true) '-App: Eleva-E-Roda pede UAC mesmo sem terminal'
Assert ($script:startCalls -eq 1) '-App: o UAC foi de fato pedido'
$App = $false
Assert ((Eleva-E-Roda 'liberar a porta' 'exit 0') -eq $false) 'sem -App e sem terminal: Eleva-E-Roda recusa'
Assert ($script:startCalls -eq 1) 'sem -App: nenhum UAC pedido'

# Marcas de etapa
$script:currentStep = ''
$App = $false; Limpa; Mark-Step 'preparar' 'fazendo'
Assert ($script:saida.Count -eq 0) 'sem -App: nenhuma marca de etapa'
$App = $true; Limpa; Mark-Step 'preparar' 'fazendo'; Mark-Step 'preparar' 'ok'
Assert (($script:saida -join '|') -eq '##HANGAR-PASSO## preparar fazendo|##HANGAR-PASSO## preparar ok') '-App: marcas de etapa na sintaxe do contrato'
Assert ($script:currentStep -eq 'preparar') 'fazendo guarda a etapa que vai no PASSO falhou'

# Guardas no texto
$texto = [IO.File]::ReadAllText($installer)
Assert ($texto.Contains('if ($vivo -and -not $Update -and -not $App) {')) '-App nao abre app nem navegador no fim'
Assert ($texto.Contains('if ($tokenFim -and $App) {')) '-App nao imprime o token no resumo'
Assert ($texto.Contains('Remove-Item Env:HANGAR_TOKEN')) 'HANGAR_TOKEN sai do ambiente'
Assert ($texto.Contains('Write-Host "##HANGAR-FIM## $($script:finalState)"')) 'a FIM sai no finally'
$bytes = [IO.File]::ReadAllBytes($installer)
Assert ($bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) 'install.ps1 continua UTF-8 com BOM'

# Senha do celular: o que o python-dotenv reinterpreta e recusado no -App e na pergunta do terminal
$def = $ast.Find({ param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Test-ForbiddenToken' }, $true)
if (-not $def) { throw 'Funcao ausente: Test-ForbiddenToken' }
. ([scriptblock]::Create($def.Extent.Text))
foreach ($ruim in @('senha#forte', 'senha$forte', "senha'forte", 'senha"forte', 'senha\forte', ' senhaforte', "senhaforte`t")) {
    Assert (Test-ForbiddenToken $ruim) "senha recusada: [$ruim]"
}
Assert (-not (Test-ForbiddenToken 'senha boa 123')) 'espaco no meio continua valendo'
Assert ($texto.Contains('if (Test-ForbiddenToken $token) { Erro $tokenForbiddenText; continue }')) 'a pergunta do terminal recusa os mesmos caracteres'

$tokenIf = $ast.Find({ param($n) $n -is [Management.Automation.Language.IfStatementAst] -and $n.Clauses[0].Item1.Extent.Text -eq '$temToken' }, $true)
$tokenForbiddenText = 'FRASE-PROIBIDA'
$script:gravado = $null
$script:pare = $null
function Set-EnvKey { param($Chave, $Valor) $script:gravado = $Valor }
function Pare($mensagem, $dicas) { $script:pare = $mensagem; throw 'PARE' }
foreach ($n in @('Token-Aleatorio')) {
    $d = $ast.Find({ param($x) $x -is [Management.Automation.Language.FunctionDefinitionAst] -and $x.Name -eq $n }, $true)
    . ([scriptblock]::Create($d.Extent.Text))
}
function Run-TokenStep($valor) {
    $script:gravado = $null; $script:pare = $null; Limpa
    $temToken = $false; $App = $true; $tokenDoApp = $valor
    try { . ([scriptblock]::Create($tokenIf.Extent.Text)) } catch { if ("$_" -ne 'PARE') { throw } }
}
Run-TokenStep 'senha#forte'
Assert ($script:pare -and $script:pare.Contains('FRASE-PROIBIDA') -and -not $script:gravado) '-App: senha com # para a instalacao sem gravar'
Assert (-not $script:pare.Contains('senha#forte') -and -not ($script:saida -join '|').Contains('senha#forte')) '-App: a senha recusada nao aparece na saida'
Run-TokenStep 'curta'
Assert ($script:pare -and -not $script:gravado) '-App: senha curta para a instalacao'
Run-TokenStep 'senha boa 123'
Assert (-not $script:pare -and $script:gravado -eq 'senha boa 123') '-App: senha valida gravada como veio'
Run-TokenStep ''
Assert (-not $script:pare -and $script:gravado -match '^[0-9a-f]{48}$') '-App: senha vazia gera a aleatoria'

# Roda do mouse: no -App o psmux so e reiniciado com -ConsertarRoda
$askIf = $ast.Find({ param($n) $n -is [Management.Automation.Language.IfStatementAst] -and $n.Clauses[0].Item1.Extent.Text -eq "`$script:psmuxConsole -eq 'perguntar'" }, $true)
$runIf = $ast.Find({ param($n) $n -is [Management.Automation.Language.IfStatementAst] -and $n.Clauses[0].Item1.Extent.Text -eq "`$script:psmuxConsole -eq 'pular'" }, $true)
$script:conptyCalls = 0
function FakePowerShell { if ("$args" -match 'install-psmux-conpty\.ps1') { $script:conptyCalls++ }; $global:LASTEXITCODE = 1 }
# O ramo 'fazer' so encerra o psmux se o console instalou; o duble garante que nada real e derrubado.
function Psmux-Encerrar-Antigos { throw 'Psmux-Encerrar-Antigos chamado no teste' }
$PowerShellExe = 'FakePowerShell'
$raiz = 'C:\hangar-falso'
$script:psmuxSessoes = 2
$App = $true
foreach ($caso in @(@{ roda = $false; esperado = 0 }, @{ roda = $true; esperado = 1 })) {
    $ConsertarRoda = $caso.roda; $script:conptyCalls = 0; $script:faltaRodaPsmux = $false; Limpa
    $script:psmuxConsole = 'perguntar'
    . ([scriptblock]::Create($askIf.Extent.Text))
    . ([scriptblock]::Create($runIf.Extent.Text))
    Assert ($script:conptyCalls -eq $caso.esperado) "-App -ConsertarRoda:$($caso.roda): install-psmux-conpty.ps1 chamado $($caso.esperado) vez(es)"
}

# Preambulo do -App: a codepage dos .cmd e lida antes da troca para UTF-8, e o Python sai em UTF-8
$iCp = $texto.IndexOf('$script:launcherCodePage = [Console]::OutputEncoding.CodePage')
$iUtf = $texto.IndexOf('try { [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false } catch { }')
Assert ($iCp -ge 0 -and $iCp -lt $iUtf) 'a codepage dos .cmd e guardada antes da troca para UTF-8'
$appIf = $ast.Find({ param($n) $n -is [Management.Automation.Language.IfStatementAst] -and $n.Clauses[0].Item1.Extent.Text -eq '$App' -and $n.Extent.Text.Contains('HANGAR-PROTOCOLO') }, $true)
Assert ($appIf -and $appIf.Extent.Text.Contains("`$env:PYTHONIOENCODING = 'utf-8'")) '-App: o Python escreve em UTF-8'
Assert ($appIf -and $appIf.Extent.Text.Contains("`$script:launcherCodePage = [int](Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Nls\CodePage').OEMCP")) '-App: os .cmd vao na OEM do sistema, nao na do console herdado'
$d = $ast.Find({ param($x) $x -is [Management.Automation.Language.FunctionDefinitionAst] -and $x.Name -eq 'Escrever-Lancador' }, $true)
. ([scriptblock]::Create($d.Extent.Text))
$cmdFile = Join-Path $env:TEMP ("hangar-test-" + [guid]::NewGuid().ToString('N') + '.cmd')
$encAnt = [Console]::OutputEncoding
try {
    $script:launcherCodePage = 850
    [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
    [void](Escrever-Lancador $cmdFile ("@echo C:\Users\Jo" + [char]0x00E3 + "o") -Tipo cmd)
    $b = [IO.File]::ReadAllBytes($cmdFile)
    Assert ($b -contains 0xC6 -and -not ($b -contains 0xC3)) '-App: o .cmd sai na codepage guardada (850), nao em UTF-8'
} finally {
    [Console]::OutputEncoding = $encAnt
    Remove-Item $cmdFile -ErrorAction SilentlyContinue
}

# Outra instalacao segurando a trava: a FIM continua sendo a ultima linha
$mutex = New-Object Threading.Mutex($false, 'Local\HangarInstall')
$segurou = $false
try { $segurou = $mutex.WaitOne(0) } catch [Threading.AbandonedMutexException] { $segurou = $true }
if ($segurou) {
    $out = Join-Path $env:TEMP ("hangar-test-" + [guid]::NewGuid().ToString('N') + '.txt')
    try {
        & cmd /c "powershell -NoProfile -ExecutionPolicy Bypass -File `"$installer`" -App -Tailscale nao > `"$out`" 2>&1 < NUL"
        $linhas = @(Get-Content $out | Where-Object { $_.Trim() })
        Assert ($linhas.Count -and $linhas[-1] -eq '##HANGAR-FIM## falhou') "-App com a trava ocupada: ultima linha FIM falhou (veio: $($linhas[-1]))"
        Assert (@($linhas | Where-Object { $_ -match 'Outra instalacao' }).Count -eq 1) '-App com a trava ocupada: o motivo aparece'
    } finally {
        $mutex.ReleaseMutex()
        Remove-Item $out -ErrorAction SilentlyContinue
    }
} else {
    [Console]::WriteLine('skip trava ocupada: ha uma instalacao de verdade rodando')
}
$mutex.Dispose()

# --- fim dos casos ---
if ($script:falhas) { [Console]::WriteLine("$($script:falhas) falha(s)"); exit 1 }
[Console]::WriteLine('tudo ok')
