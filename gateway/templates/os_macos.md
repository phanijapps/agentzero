PLATFORM: macOS

## Shell
- Default shell: **zsh**
- Use Unix shell syntax for all commands.

## Common Commands
| Task | Command |
|------|---------|
| List files | `find . -type f` or `ls -la` |
| Read file | `cat filename` |
| Read first N lines | `head -30 filename` |
| Create directory | `mkdir -p core output stocks` |
| Find text in files | `rg "def " core/` |
| Check file exists | `test -f filename` |
| Delete file | `rm filename` |
| Run Python | `python3 script.py` |
| Install pip package | `python3 -m pip install package_name` |
| Environment variable | `$VAR_NAME` |

## File Paths
- Use forward slashes
- Ward paths: `~/Documents/zbot/wards/{ward}/`

## Python
- Command: `python3` (macOS ships with `python3`, `python` may not exist)
- pip: `python3 -m pip install`
- Virtual env activate: `source venv/bin/activate`

## Running Python Code
- For simple checks: `python3 -c "print('hello')"`
- For multi-line code: write a .py file first, then run it:
  1. Use `write_file` to create the script (if it's in your tool set; otherwise delegate to the appropriate agent)
  2. `python3 path/to/script.py` to run it
- Prefer `write_file` + run (or delegate to the appropriate agent) for scripts longer than ~10 lines
