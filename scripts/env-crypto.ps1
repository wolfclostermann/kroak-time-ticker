<#
.SYNOPSIS
  Encrypt or decrypt .env for committing a ciphertext blob (.env.enc) to GitHub.

  PowerShell port of scripts/env-crypto.sh. Produces/reads the same on-disk
  format as `openssl enc -aes-256-cbc -salt -pbkdf2 -iter 600000` (8-byte
  "Salted__" magic + 8-byte salt + ciphertext; key+IV derived via
  PBKDF2-HMAC-SHA256) so files encrypted with either script can be decrypted
  by the other.

.PARAMETER Action
  'encrypt' (reads .env, writes .env.enc) or 'decrypt' (reads .env.enc, writes .env).

.PARAMETER EnvFile
  Plaintext path (default: .env). Ciphertext path is "<EnvFile>.enc".

.EXAMPLE
  ./scripts/env-crypto.ps1 encrypt
.EXAMPLE
  ./scripts/env-crypto.ps1 decrypt
.EXAMPLE
  ./scripts/env-crypto.ps1 encrypt -EnvFile my.env   # writes my.env.enc

.NOTES
  Passphrase is read from the terminal (not stored in the repo).
  Requires PowerShell with .NET support for Rfc2898DeriveBytes + SHA256
  (PowerShell 7+, or Windows PowerShell 5.1 on .NET Framework 4.7.2+).
#>
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [ValidateSet('encrypt', 'decrypt')]
    [string]$Action,

    [string]$EnvFile = ".env"
)

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# OpenSSL-compatible defaults; adjust Iterations only if you change it here
# and in scripts/env-crypto.sh.
$Iterations = 600000
$EncFile    = "$EnvFile.enc"
$SaltMagic  = [System.Text.Encoding]::ASCII.GetBytes("Salted__")  # 8 bytes

function Read-PassphraseBytes([string]$Prompt) {
    $secure = Read-Host -Prompt $Prompt -AsSecureString
    $bstr = [System.Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
    try {
        $plain = [System.Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr)
        return [System.Text.Encoding]::UTF8.GetBytes($plain)
    } finally {
        [System.Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr)
    }
}

# Returns @(key, iv) — 32-byte AES-256 key + 16-byte CBC IV, PBKDF2-HMAC-SHA256.
function Get-KeyAndIV([byte[]]$PassBytes, [byte[]]$Salt) {
    $pbkdf2 = [System.Security.Cryptography.Rfc2898DeriveBytes]::new(
        $PassBytes, $Salt, $Iterations, [System.Security.Cryptography.HashAlgorithmName]::SHA256)
    try {
        $keyIv = $pbkdf2.GetBytes(48)
        return ,@($keyIv[0..31], $keyIv[32..47])
    } finally {
        $pbkdf2.Dispose()
    }
}

switch ($Action) {
    'encrypt' {
        if (-not (Test-Path $EnvFile)) {
            Write-Error "$EnvFile not found (create it or pass -EnvFile ...)"
            exit 1
        }

        $pass1 = Read-PassphraseBytes "Passphrase"
        $pass2 = Read-PassphraseBytes "Passphrase (again)"
        $match = ($pass1.Length -eq $pass2.Length) -and
                 (-not (Compare-Object $pass1 $pass2 -SyncWindow 0))
        if (-not $match) {
            Write-Error "passphrases do not match"
            exit 1
        }

        $salt = [byte[]]::new(8)
        [System.Security.Cryptography.RandomNumberGenerator]::Fill($salt)
        $keyIv = Get-KeyAndIV $pass1 $salt

        $plaintext = [System.IO.File]::ReadAllBytes((Resolve-Path $EnvFile))

        $aes = [System.Security.Cryptography.Aes]::Create()
        $aes.Mode = [System.Security.Cryptography.CipherMode]::CBC
        $aes.Padding = [System.Security.Cryptography.PaddingMode]::PKCS7
        $aes.Key = $keyIv[0]
        $aes.IV  = $keyIv[1]
        try {
            $encryptor = $aes.CreateEncryptor()
            $ciphertext = $encryptor.TransformFinalBlock($plaintext, 0, $plaintext.Length)
        } finally {
            $aes.Dispose()
        }

        $outBytes = New-Object byte[] ($SaltMagic.Length + $salt.Length + $ciphertext.Length)
        [Array]::Copy($SaltMagic, 0, $outBytes, 0, $SaltMagic.Length)
        [Array]::Copy($salt, 0, $outBytes, $SaltMagic.Length, $salt.Length)
        [Array]::Copy($ciphertext, 0, $outBytes, $SaltMagic.Length + $salt.Length, $ciphertext.Length)
        [System.IO.File]::WriteAllBytes((Join-Path $root $EncFile), $outBytes)

        Write-Host "Wrote $EncFile — you can commit that file. Never commit the passphrase."
    }

    'decrypt' {
        if (-not (Test-Path $EncFile)) {
            Write-Error "$EncFile not found"
            exit 1
        }

        $pass = Read-PassphraseBytes "Passphrase"

        $data = [System.IO.File]::ReadAllBytes((Resolve-Path $EncFile))
        if ($data.Length -lt 16 -or
            [System.Text.Encoding]::ASCII.GetString($data[0..7]) -ne "Salted__") {
            Write-Error "$EncFile does not look like an openssl-salted ciphertext"
            exit 1
        }
        $salt       = $data[8..15]
        $ciphertext = if ($data.Length -gt 16) { $data[16..($data.Length - 1)] } else { [byte[]]::new(0) }

        $keyIv = Get-KeyAndIV $pass $salt

        $aes = [System.Security.Cryptography.Aes]::Create()
        $aes.Mode = [System.Security.Cryptography.CipherMode]::CBC
        $aes.Padding = [System.Security.Cryptography.PaddingMode]::PKCS7
        $aes.Key = $keyIv[0]
        $aes.IV  = $keyIv[1]
        try {
            $decryptor = $aes.CreateDecryptor()
            $plaintext = $decryptor.TransformFinalBlock($ciphertext, 0, $ciphertext.Length)
        } catch {
            Write-Error "decryption failed — wrong passphrase?"
            exit 1
        } finally {
            $aes.Dispose()
        }

        [System.IO.File]::WriteAllBytes((Join-Path $root $EnvFile), $plaintext)
        Write-Host "Wrote $EnvFile."
    }
}
