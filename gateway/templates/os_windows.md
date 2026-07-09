PLATFORM: Windows

## Shell
- Default shell: **PowerShell**
- Use PowerShell syntax for all shell commands. Do NOT use bash syntax.

## Common Commands
| Task | Command |
|------|---------|
| List files | `Get-ChildItem -Recurse -File` or `dir /s /b` |
| Read file | `Get-Content filename` |
| Read first N lines | `Get-Content filename -TotalCount 30` |
| Create directory | `New-Item -ItemType Directory -Force -Path core,output,stocks` |
| Find text in files | `Select-String -Path "core\*.py" -Pattern "def "` |
| Check file exists | `Test-Path filename` |
| Delete file | `Remove-Item filename` |
| Run Python | `python script.py` (NOT `python3`) |
| Install pip package | `python -m pip install package_name` |
| Environment variable | `$env:VAR_NAME` |

## Avoid These (bash-only, will fail)
- `mkdir -p` → use `New-Item -ItemType Directory -Force`
- `find . -type f` → use `Get-ChildItem -Recurse -File`
- `head -30 file` → use `Get-Content file -TotalCount 30`
- `ls -la` → use `Get-ChildItem` or `dir`
- `cat file | grep` → use `Select-String`
- `&&` chaining → use `;` or separate commands
- Heredocs (`<< 'EOF'`) → use `write_file` for file creation / `edit_file` for edits if they're in your tool set; otherwise delegate to the appropriate agent
- `python3` → use `python`

## File Paths
- Use backslashes or forward slashes (both work in Python)
- Ward paths: `C:\Users\{user}\Documents\zbot\wards\{ward}\`

## Python
- Command: `python` (NOT `python3`)
- pip: `python -m pip install`
- Virtual env activate: `.\venv\Scripts\Activate.ps1`

- For simple checks: `python3 -c "print('hello')"`
- For multi-line code: write a .py file first, then run it:
  1. Use `write_file` to create the script (if it's in your tool set; otherwise delegate to the appropriate agent)
  2. `python3 path/to/script.py` to run it
- Prefer `write_file` + run (or delegate to the appropriate agent) for scripts longer than ~10 lines
