# Windows CI setup and functional ordinary-user provider evidence.
# Requires PROBE_TARGET, PROBE_ARCH, GITHUB_WORKSPACE and RUNNER_TEMP.
# Provision accounts only on an ephemeral CI runner.

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$evidence = Join-Path $env:GITHUB_WORKSPACE '.dev/out/ordinary-user'
New-Item -ItemType Directory -Path $evidence -Force | Out-Null
'Functional ordinary-user candidate evidence only; no power-loss or production qualification claim.' |
  Set-Content (Join-Path $evidence 'scope.txt')
$PSVersionTable.PSVersion.ToString() | Set-Content (Join-Path $evidence 'powershell-version.txt')
rustc -vV | Set-Content (Join-Path $evidence 'rustc.txt')
if ($LASTEXITCODE -ne 0) { throw 'rustc identity failed' }
rustup target add $env:PROBE_TARGET
if ($LASTEXITCODE -ne 0) { throw 'native target setup failed' }
$artifacts = & cargo test --locked -p morphir-package --test provider_qualification `
  --target $env:PROBE_TARGET --no-run --message-format=json
if ($LASTEXITCODE -ne 0) { throw 'provider probe build failed' }
$executables = @($artifacts | ForEach-Object { $_ | ConvertFrom-Json } |
  Where-Object { $_.reason -eq 'compiler-artifact' } |
  Where-Object { $_.target.name -eq 'provider_qualification' -and $_.profile.test -and $_.executable } |
  ForEach-Object { $_.executable })
if ($executables.Count -ne 1) { throw 'expected exactly one provider test executable' }
$setupSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$accountName = 'morphir' + [Guid]::NewGuid().ToString('N').Substring(0, 12)
$sandbox = Join-Path $env:RUNNER_TEMP $accountName
$account = $null
$secret = $null
$credential = $null
try {
  # Account creation belongs to ephemeral CI setup, never to the provider.
  $password = [Convert]::ToBase64String([Security.Cryptography.RandomNumberGenerator]::GetBytes(32)) + 'aA1!'
  $secret = ConvertTo-SecureString $password -AsPlainText -Force
  $password = $null
  $account = New-LocalUser -Name $accountName -Password $secret -Description 'Ephemeral Morphir provider probe'
  Add-LocalGroupMember -SID 'S-1-5-32-545' -Member $account
  $memberships = @(Get-LocalGroup | Where-Object {
    @(Get-LocalGroupMember -SID $_.SID | Where-Object { $_.SID -eq $account.SID }).Count -gt 0
  } | ForEach-Object { $_.SID.Value })
  if ($memberships.Count -ne 1 -or $memberships[0] -ne 'S-1-5-32-545') {
    throw 'probe account must belong only to local Users'
  }
  New-Item -ItemType Directory -Path $sandbox | Out-Null
  $scratch = New-Item -ItemType Directory -Path (Join-Path $sandbox 'scratch')
  function Set-ProbeAcl($path, $userRights) {
    $acl = [Security.AccessControl.DirectorySecurity]::new()
    $acl.SetAccessRuleProtection($true, $false)
    $acl.SetOwner([Security.Principal.SecurityIdentifier]::new($setupSid))
    foreach ($sid in @($setupSid, 'S-1-5-18')) {
      $rule = [Security.AccessControl.FileSystemAccessRule]::new(
        [Security.Principal.SecurityIdentifier]::new($sid), 'FullControl',
        'ContainerInherit,ObjectInherit', 'None', 'Allow')
      $acl.AddAccessRule($rule)
    }
    $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
      $account.SID, $userRights, 'ContainerInherit,ObjectInherit', 'None', 'Allow'))
    Set-Acl -LiteralPath $path -AclObject $acl
  }
  Set-ProbeAcl $sandbox 'ReadAndExecute'
  Set-ProbeAcl $scratch.FullName 'FullControl'
  & icacls $scratch.FullName /setowner ('*' + $account.SID.Value)
  if ($LASTEXITCODE -ne 0) { throw 'scratch ownership setup failed' }
  $scratchAcl = Get-Acl -LiteralPath $scratch.FullName
  if ($scratchAcl.GetOwner([Security.Principal.SecurityIdentifier]).Value -ne $account.SID.Value -or
      -not $scratchAcl.AreAccessRulesProtected) { throw 'scratch ownership/ACL readback failed' }
  @{
    accountSid = $account.SID.Value; setupSid = $setupSid; localGroups = $memberships
    scratchOwner = $scratchAcl.Owner; scratchSddl = $scratchAcl.Sddl
    expectedArchitecture = $env:PROBE_ARCH
    osVersion = [Environment]::OSVersion.VersionString
  } | ConvertTo-Json | Set-Content (Join-Path $evidence 'setup.json')
  $executable = Join-Path $sandbox 'provider_qualification.exe'
  Copy-Item -LiteralPath $executables[0] -Destination $executable
  $executableAcl = Get-Acl -LiteralPath $executable
  $executableAcl.SetOwner([Security.Principal.SecurityIdentifier]::new($setupSid))
  Set-Acl -LiteralPath $executable -AclObject $executableAcl
  $originalHash = (Get-FileHash -LiteralPath $executables[0] -Algorithm SHA256).Hash
  $copiedHash = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash
  if ($copiedHash -ne $originalHash) { throw 'probe executable copy changed' }
  $copiedHash | Set-Content (Join-Path $evidence 'executable-sha256.txt')
  $credential = [PSCredential]::new("$env:COMPUTERNAME\$accountName", $secret)
  # Start-Process discards -Environment with credentialed -UseNewEnvironment.
  # Explicit null entries remove inherited runner values before our allowlist.
  $childEnvironment = @{}
  foreach ($name in [Environment]::GetEnvironmentVariables().Keys) {
    $childEnvironment[$name] = $null
  }
  $probeEnvironment = @{
    TEMP = $scratch.FullName; TMP = $scratch.FullName
    PATH = "$env:SystemRoot\System32"; SystemRoot = $env:SystemRoot
    MORPHIR_PROVIDER_STANDARD_USER = '1'; MORPHIR_PROVIDER_EXPECTED_SID = $account.SID.Value
    MORPHIR_PROVIDER_SETUP_SID = $setupSid; MORPHIR_PROVIDER_EXPECTED_ARCH = $env:PROBE_ARCH
  }
  foreach ($entry in $probeEnvironment.GetEnumerator()) {
    $childEnvironment[$entry.Key] = $entry.Value
  }
  # Fresh local logon, not a filtered admin token or network-only credentials.
  $process = Start-Process -FilePath $executable -Credential $credential -LoadUserProfile `
    -WorkingDirectory $sandbox -Environment $childEnvironment `
    -ArgumentList '--include-ignored --nocapture --test-threads=1' -Wait -PassThru `
    -RedirectStandardOutput (Join-Path $evidence 'stdout.log') `
    -RedirectStandardError (Join-Path $evidence 'stderr.log')
  Get-Content (Join-Path $evidence 'stdout.log')
  Get-Content (Join-Path $evidence 'stderr.log')
  if ($process.ExitCode -ne 0) { throw "standard-user probe exited $($process.ExitCode)" }
  $testOutput = Get-Content (Join-Path $evidence 'stdout.log') -Raw
  if ($testOutput -notmatch '(?m)^test ordinary_user_identity_and_state_acl_are_observed \.\.\. ok\r?$' -or
      $testOutput -notmatch 'test result: ok\. [1-9][0-9]* passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;') {
    throw 'required ordinary-user case or complete unfiltered test run was not observed'
  }
  if ((Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash -ne $copiedHash) {
    throw 'probe executable changed during execution'
  }
} finally {
  $credential = $null
  if ($null -ne $secret) { $secret.Dispose() }
  $cleanupErrors = [Collections.Generic.List[string]]::new()
  if ($null -ne $account) {
    try {
      Get-CimInstance Win32_UserProfile -Filter "SID='$($account.SID.Value)'" |
        Remove-CimInstance -ErrorAction Stop
    } catch { $cleanupErrors.Add('profile cleanup failed') }
    try { Remove-LocalUser -SID $account.SID -ErrorAction Stop }
    catch { $cleanupErrors.Add('account cleanup failed') }
  }
  try {
    if (Test-Path -LiteralPath $sandbox) { Remove-Item -LiteralPath $sandbox -Recurse -Force }
  } catch { $cleanupErrors.Add('sandbox cleanup failed') }
  if ($cleanupErrors.Count) { throw ($cleanupErrors -join '; ') }
}
