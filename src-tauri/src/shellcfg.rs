//! Shell setup: one clearly marked block in ~/.zshrc or ~/.bashrc. Nothing outside the markers is ever changed,
//! the first original is backed up, and the block can be removed again.
use crate::exec::{self, cancelled, log, step, stream};
use crate::system;
use serde::{Deserialize, Serialize};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter};

const BEGIN: &str = "# >>> devsetup shell >>>";
const END: &str = "# <<< devsetup shell <<<";
const DAMAGED: &str = "The DevSetup block in this file is damaged (a marker is missing or duplicated). Fix or delete the marker lines by hand first.";

const AUTOSUG_PATHS: [&str; 2] = ["/usr/share/zsh-autosuggestions/zsh-autosuggestions.zsh", "/usr/share/zsh/plugins/zsh-autosuggestions/zsh-autosuggestions.zsh"];
const SYNTAX_PATHS: [&str; 2] = ["/usr/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh", "/usr/share/zsh/plugins/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh"];

const HIST_ZSH: &str = r#"# History
HISTFILE="$HOME/.zsh_history"
HISTSIZE=50000
SAVEHIST=50000
setopt APPEND_HISTORY SHARE_HISTORY HIST_IGNORE_DUPS HIST_IGNORE_SPACE HIST_REDUCE_BLANKS"#;
const HIST_BASH: &str = r#"# History
HISTSIZE=50000
HISTFILESIZE=100000
HISTCONTROL=ignoreboth:erasedups
shopt -s histappend"#;
const COMP_ZSH: &str = r#"# Completion
autoload -Uz compinit && compinit
zstyle ':completion:*' menu select
zstyle ':completion:*' matcher-list 'm:{a-z}={A-Za-z}'"#;
const COMP_BASH: &str = r#"# Completion
if ! shopt -oq posix; then
  [ -r /usr/share/bash-completion/bash_completion ] && . /usr/share/bash-completion/bash_completion
fi"#;
const FZF_ZSH: &str = r#"# fzf: Ctrl+R history search, Ctrl+T file search, Alt+C change directory
if [ -r /usr/share/doc/fzf/examples/key-bindings.zsh ]; then
  source /usr/share/doc/fzf/examples/key-bindings.zsh
  [ -r /usr/share/doc/fzf/examples/completion.zsh ] && source /usr/share/doc/fzf/examples/completion.zsh
elif command -v fzf >/dev/null 2>&1 && fzf --zsh >/dev/null 2>&1; then
  source <(fzf --zsh)
fi"#;
const FZF_BASH: &str = r#"# fzf: Ctrl+R history search, Ctrl+T file search, Alt+C change directory
if [ -r /usr/share/doc/fzf/examples/key-bindings.bash ]; then
  . /usr/share/doc/fzf/examples/key-bindings.bash
elif command -v fzf >/dev/null 2>&1 && fzf --bash >/dev/null 2>&1; then
  eval "$(fzf --bash)"
fi"#;
const ALIASES: &str = r#"# Aliases
command -v batcat >/dev/null 2>&1 && ! command -v bat >/dev/null 2>&1 && alias bat=batcat
command -v fdfind >/dev/null 2>&1 && ! command -v fd >/dev/null 2>&1 && alias fd=fdfind
if command -v eza >/dev/null 2>&1; then
  alias ls='eza --group-directories-first'
  alias ll='eza -l --group-directories-first --git'
  alias la='eza -la --group-directories-first --git'
else
  alias ll='ls -alF'
  alias la='ls -A'
fi
alias ..='cd ..'
alias ...='cd ../..'
alias grep='grep --color=auto'"#;
const GIT_ALIASES: &str = r#"# Git aliases
alias gs='git status -sb'
alias gd='git diff'
alias gl='git log --oneline --graph --decorate -20'
alias ga='git add'
alias gc='git commit'
alias gco='git checkout'
alias gpl='git pull --ff-only'
alias gps='git push'"#;
const AUTOSUGGESTIONS: &str = r#"# zsh-autosuggestions
for f in /usr/share/zsh-autosuggestions/zsh-autosuggestions.zsh /usr/share/zsh/plugins/zsh-autosuggestions/zsh-autosuggestions.zsh; do
  [ -r "$f" ] && { source "$f"; break; }
done"#;
const SYNTAX_HL: &str = r#"# zsh-syntax-highlighting (keep this last)
for f in /usr/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh /usr/share/zsh/plugins/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh; do
  [ -r "$f" ] && { source "$f"; break; }
done"#;

#[derive(Deserialize, Serialize, Default, Clone)]
#[serde(default)]
pub struct Options {
    pub starship: bool,
    pub autosuggestions: bool,
    pub syntax_highlighting: bool,
    pub history: bool,
    pub completion: bool,
    pub zoxide: bool,
    pub fzf: bool,
    pub direnv: bool,
    pub mise: bool,
    pub aliases: bool,
    pub git_aliases: bool,
}

fn eval_line(cmd: &str, init: &str) -> String { format!("command -v {cmd} >/dev/null 2>&1 && eval \"$({init})\"") }

/// The managed block for `shell` ("zsh" or "bash"). Order matters: syntax highlighting must be last in zsh.
pub fn render(shell: &str, o: &Options) -> Result<String, String> {
    let zsh = match shell { "zsh" => true, "bash" => false, _ => return Err("Choose zsh or bash.".into()) };
    let sh = if zsh { "zsh" } else { "bash" };
    let pick = |z: &'static str, b: &'static str| -> &'static str { if zsh { z } else { b } };
    let mut parts: Vec<String> = vec![];
    if o.history { parts.push(pick(HIST_ZSH, HIST_BASH).to_string()); }
    if o.completion { parts.push(pick(COMP_ZSH, COMP_BASH).to_string()); }
    if o.mise { parts.push(eval_line("mise", &format!("mise activate {sh}"))); }
    if o.direnv { parts.push(eval_line("direnv", &format!("direnv hook {sh}"))); }
    if o.zoxide { parts.push(eval_line("zoxide", &format!("zoxide init {sh}"))); }
    if o.fzf { parts.push(pick(FZF_ZSH, FZF_BASH).to_string()); }
    if o.aliases { parts.push(ALIASES.to_string()); }
    if o.git_aliases { parts.push(GIT_ALIASES.to_string()); }
    if zsh && o.autosuggestions { parts.push(AUTOSUGGESTIONS.to_string()); }
    if o.starship { parts.push(eval_line("starship", &format!("starship init {sh}"))); }
    if zsh && o.syntax_highlighting { parts.push(SYNTAX_HL.to_string()); }
    if parts.is_empty() { return Err("Tick at least one option.".into()); }
    Ok(format!(
        "{BEGIN}\n# Managed by DevSetup. Applying again replaces everything between these markers.\n# To undo it, use Remove in DevSetup (Shell setup) or delete these lines.\n{}\n{END}\n",
        parts.iter().map(|p| p.trim_end()).collect::<Vec<_>>().join("\n\n")
    ))
}

/// Things in the user's own part of the file that probably do the same job (a heuristic warning only).
pub fn duplicates(outside: &str, o: &Options) -> Vec<String> {
    let checks = [
        (o.starship, "starship init", "the Starship prompt"), (o.zoxide, "zoxide init", "zoxide"), (o.direnv, "direnv hook", "direnv"),
        (o.mise, "mise activate", "mise"), (o.autosuggestions, "zsh-autosuggestions", "zsh-autosuggestions"),
        (o.syntax_highlighting, "zsh-syntax-highlighting", "zsh-syntax-highlighting"), (o.completion, "compinit", "completion"), (o.history, "HISTSIZE", "history settings"),
    ];
    checks.iter()
        .filter(|(on, pat, _)| *on && outside.lines().any(|l| { let t = l.trim_start(); !t.starts_with('#') && t.contains(*pat) }))
        .map(|(_, _, name)| format!("{name} already seems to be set up elsewhere in this file"))
        .collect()
}

// ---------- the managed block ----------

/// Byte range of the whole block (both marker lines included), None when there is none.
fn find_block(text: &str) -> Result<Option<(usize, usize)>, String> {
    let (mut begin, mut end, mut pos, mut nb, mut ne) = (None, None, 0usize, 0, 0);
    for line in text.split_inclusive('\n') {
        let t = line.trim_end();
        if t == BEGIN { nb += 1; if begin.is_none() { begin = Some(pos); } }
        if t == END { ne += 1; end = Some(pos + line.len()); }
        pos += line.len();
    }
    match (nb, ne, begin, end) {
        (0, 0, _, _) => Ok(None),
        (1, 1, Some(b), Some(e)) if b < e => Ok(Some((b, e))),
        _ => Err(DAMAGED.to_string()),
    }
}

pub fn apply_block(existing: &str, block: &str) -> Result<String, String> {
    Ok(match find_block(existing)? {
        Some((b, e)) => format!("{}{}{}", &existing[..b], block, &existing[e..]),
        None => {
            let mut s = existing.to_string();
            if !s.is_empty() { if !s.ends_with('\n') { s.push('\n'); } s.push('\n'); }
            s.push_str(block);
            s
        }
    })
}

/// The file without the block. None when there was no block.
pub fn remove_block(existing: &str) -> Result<Option<String>, String> {
    Ok(find_block(existing)?.map(|(b, e)| {
        let mut before = existing[..b].to_string();
        if before.ends_with("\n\n") { before.pop(); }
        format!("{before}{}", &existing[e..])
    }))
}

// ---------- files ----------

fn rc_path(shell: &str) -> Result<PathBuf, String> {
    let home = system::home().ok_or_else(|| "no home folder".to_string())?;
    match shell { "zsh" => Ok(home.join(".zshrc")), "bash" => Ok(home.join(".bashrc")), _ => Err("Choose zsh or bash.".to_string()) }
}

/// A symlinked rc file (for example from a dotfiles repository) is edited where it points, so the link survives.
fn real_target(path: &Path) -> PathBuf {
    if std::fs::symlink_metadata(path).map(|m| m.file_type().is_symlink()).unwrap_or(false) { std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()) } else { path.to_path_buf() }
}

fn write_atomic(target: &Path, content: &str) -> Result<(), String> {
    let dir = target.parent().ok_or_else(|| "no parent folder".to_string())?;
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.devsetup-tmp"));
    let mode = std::fs::metadata(target).map(|m| m.permissions().mode() & 0o777).unwrap_or(0o644);
    let result = std::fs::write(&tmp, content)
        .and_then(|_| std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode)))
        .and_then(|_| std::fs::rename(&tmp, target));
    if let Err(e) = result { let _ = std::fs::remove_file(&tmp); return Err(format!("Could not write {}: {e}", target.display())); }
    Ok(())
}

/// Copies the original once, before DevSetup's first change. Returns the backup path when one was made.
fn backup_once(target: &Path) -> Result<Option<PathBuf>, String> {
    let bak = PathBuf::from(format!("{}.devsetup-backup", target.display()));
    if !target.is_file() || bak.exists() { return Ok(None); }
    std::fs::copy(target, &bak).map_err(|e| format!("Could not back up {}: {e}", target.display()))?;
    Ok(Some(bak))
}

fn read_or_empty(target: &Path) -> Result<String, String> {
    if !target.exists() { return Ok(String::new()); }
    std::fs::read_to_string(target).map_err(|e| format!("Could not read {}: {e}", target.display()))
}

#[derive(Serialize, Default)]
pub struct Preview { pub path: String, pub exists: bool, pub had_block: bool, pub block: String, pub warnings: Vec<String> }

pub fn preview(shell: &str, o: &Options) -> Result<Preview, String> {
    let block = render(shell, o)?;
    let path = rc_path(shell)?;
    let target = real_target(&path);
    let existing = read_or_empty(&target)?;
    let had_block = find_block(&existing)?.is_some();
    let outside = remove_block(&existing)?.unwrap_or(existing);
    Ok(Preview { path: path.to_string_lossy().into_owned(), exists: target.exists(), had_block, block, warnings: duplicates(&outside, o) })
}

pub fn apply(shell: &str, o: &Options) -> Result<String, String> {
    let block = render(shell, o)?;
    let path = rc_path(shell)?;
    let target = real_target(&path);
    let updated = apply_block(&read_or_empty(&target)?, &block)?;
    let bak = backup_once(&target)?;
    write_atomic(&target, &updated)?;
    let saved = bak.map(|b| format!(" Your original is saved as {}.", b.display())).unwrap_or_default();
    Ok(format!("Updated {}.{saved} Open a new terminal to use it.", path.display()))
}

pub fn remove(shell: &str) -> Result<String, String> {
    let path = rc_path(shell)?;
    let target = real_target(&path);
    let existing = read_or_empty(&target)?;
    let Some(updated) = remove_block(&existing)? else { return Err(format!("DevSetup has not added anything to {}.", path.display())); };
    backup_once(&target)?;
    write_atomic(&target, &updated)?;
    Ok(format!("Removed the DevSetup block from {}. Open a new terminal to see the change.", path.display()))
}

// ---------- state ----------

#[derive(Serialize, Default)]
pub struct RcInfo { pub shell: String, pub path: String, pub exists: bool, pub has_block: bool, pub damaged: Option<String> }

#[derive(Serialize, Default)]
pub struct Available {
    pub starship: bool, pub zoxide: bool, pub fzf: bool, pub direnv: bool, pub mise: bool, pub eza: bool, pub bat: bool, pub fd: bool,
    pub autosuggestions: bool, pub syntax_highlighting: bool,
}

#[derive(Serialize, Default)]
pub struct ShellState { pub login_shell: String, pub env_shell: String, pub zsh_installed: bool, pub bash_installed: bool, pub files: Vec<RcInfo>, pub available: Available }

/// Login shell of `uid` from the contents of /etc/passwd.
fn login_shell_of(uid: u32, passwd: &str) -> Option<String> {
    passwd.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        if f.len() >= 7 && f[2].parse::<u32>().ok() == Some(uid) { Some(f[6].to_string()) } else { None }
    })
}

pub fn state() -> ShellState {
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    let any = |paths: &[&str]| paths.iter().any(|p| Path::new(p).exists());
    let mut files = vec![];
    for shell in ["zsh", "bash"] {
        let Ok(path) = rc_path(shell) else { continue };
        let target = real_target(&path);
        let text = std::fs::read_to_string(&target).unwrap_or_default();
        let (has_block, damaged) = match find_block(&text) { Ok(b) => (b.is_some(), None), Err(e) => (false, Some(e)) };
        files.push(RcInfo { shell: shell.to_string(), path: path.to_string_lossy().into_owned(), exists: target.exists(), has_block, damaged });
    }
    ShellState {
        login_shell: login_shell_of(uid, &passwd).unwrap_or_default(),
        env_shell: std::env::var("SHELL").unwrap_or_default(),
        zsh_installed: system::on_path("zsh") || Path::new("/usr/bin/zsh").exists(),
        bash_installed: system::on_path("bash"),
        files,
        available: Available {
            starship: system::on_path("starship"), zoxide: system::on_path("zoxide"), fzf: system::on_path("fzf"), direnv: system::on_path("direnv"),
            mise: system::on_path("mise"), eza: system::on_path("eza"), bat: system::on_path("bat") || system::on_path("batcat"),
            fd: system::on_path("fd") || system::on_path("fdfind"), autosuggestions: any(&AUTOSUG_PATHS), syntax_highlighting: any(&SYNTAX_PATHS),
        },
    }
}

/// Makes zsh or bash the login shell through the root helper (one password prompt).
pub fn start_login_shell(app: AppHandle, shell: String) -> Result<(), String> {
    if shell != "zsh" && shell != "bash" { return Err("Choose zsh or bash.".into()); }
    if exec::RUNNING.swap(true, Ordering::SeqCst) { return Err("a run is already in progress".into()); }
    exec::CANCEL.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        step(&app, "login-shell", "running", "");
        let mut cmd = Command::new("pkexec");
        match std::env::current_exe() {
            Ok(exe) => { cmd.arg(exe).arg("--helper").arg("shell").arg(&shell); }
            Err(e) => log(&app, "login-shell", "stderr", &format!("cannot find own executable: {e}")),
        }
        let ok = stream(&app, "login-shell", cmd, false);
        if ok { step(&app, "login-shell", "done", ""); } else { step(&app, "login-shell", "failed", "see log"); }
        let was_cancelled = cancelled();
        exec::RUNNING.store(false, Ordering::SeqCst);
        let _ = app.emit("run-done", exec::Done { ok: ok && !was_cancelled, failed: usize::from(!ok), cancelled: was_cancelled });
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Options {
        Options { starship: true, autosuggestions: true, syntax_highlighting: true, history: true, completion: true, zoxide: true, fzf: true, direnv: true, mise: true, aliases: true, git_aliases: true }
    }

    #[test]
    fn zsh_block_has_syntax_highlighting_last_and_bash_has_no_zsh_plugins() {
        let z = render("zsh", &all()).unwrap();
        assert!(z.starts_with(BEGIN) && z.trim_end().ends_with(END));
        let (hl, st, comp) = (z.find("zsh-syntax-highlighting (keep").unwrap(), z.find("starship init zsh").unwrap(), z.find("compinit").unwrap());
        assert!(comp < st && st < hl, "completion, then prompt, then syntax highlighting");
        assert!(z.rfind("source").unwrap() < z.find(END).unwrap());
        let b = render("bash", &all()).unwrap();
        assert!(b.contains("starship init bash") && b.contains("histappend"));
        assert!(!b.contains("zsh-autosuggestions") && !b.contains("compinit") && !b.contains("zsh-syntax"));
    }

    #[test]
    fn options_turn_pieces_on_and_off() {
        let only = Options { history: true, ..Default::default() };
        let z = render("zsh", &only).unwrap();
        assert!(z.contains("HISTSIZE") && !z.contains("starship") && !z.contains("alias"));
        assert!(render("zsh", &Options::default()).is_err());
        assert!(render("fish", &only).is_err());
    }

    #[test]
    fn plugin_paths_in_the_block_match_the_detection_paths() {
        for p in AUTOSUG_PATHS { assert!(AUTOSUGGESTIONS.contains(p), "{p}"); }
        for p in SYNTAX_PATHS { assert!(SYNTAX_HL.contains(p), "{p}"); }
    }

    #[test]
    fn the_block_is_added_replaced_and_removed_without_touching_the_rest() {
        let block = render("zsh", &Options { history: true, ..Default::default() }).unwrap();
        let original = "export FOO=1\nalias x=y\n";
        let added = apply_block(original, &block).unwrap();
        assert!(added.starts_with(original) && added.contains(BEGIN));
        let newer = render("zsh", &Options { zoxide: true, ..Default::default() }).unwrap();
        let replaced = apply_block(&added, &newer).unwrap();
        assert_eq!(replaced.matches(BEGIN).count(), 1);
        assert!(replaced.contains("zoxide init zsh") && !replaced.contains("HISTSIZE") && replaced.starts_with(original));
        assert_eq!(remove_block(&replaced).unwrap().unwrap(), original);
        assert_eq!(remove_block(original).unwrap(), None);
        // file without a trailing newline, and an empty file
        assert!(apply_block("export A=1", &block).unwrap().starts_with("export A=1\n\n"));
        assert_eq!(apply_block("", &block).unwrap(), block);
        // text after the block stays
        let sandwich = format!("before\n{block}after\n");
        assert_eq!(remove_block(&sandwich).unwrap().unwrap(), "before\nafter\n");
    }

    #[test]
    fn a_damaged_block_is_refused_not_guessed() {
        for bad in [format!("{BEGIN}\nstuff\n"), format!("stuff\n{END}\n"), format!("{END}\n{BEGIN}\n"), format!("{BEGIN}\n{END}\n{BEGIN}\n{END}\n")] {
            assert!(apply_block(&bad, "x\n").is_err() && remove_block(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn warns_about_setup_that_already_exists_outside_the_block() {
        let o = Options { starship: true, zoxide: true, ..Default::default() };
        let w = duplicates("# eval \"$(zoxide init zsh)\"\neval \"$(starship init zsh)\"\n", &o);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("Starship"));
    }

    #[test]
    fn files_are_written_atomically_keep_their_mode_and_symlinks_survive() {
        let base = std::env::temp_dir().join(format!("devsetup-shell-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("dotfiles")).unwrap();
        let real = base.join("dotfiles/zshrc");
        std::fs::write(&real, "export A=1\n").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = base.join(".zshrc");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let target = real_target(&link);
        assert_eq!(target, std::fs::canonicalize(&real).unwrap());
        assert!(backup_once(&target).unwrap().is_some());
        assert!(backup_once(&target).unwrap().is_none(), "only the first original is backed up");
        write_atomic(&target, "export A=2\n").unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "the link must survive");
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "export A=2\n");
        assert_eq!(std::fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_to_string(format!("{}.devsetup-backup", target.display())).unwrap(), "export A=1\n");
        assert!(!base.join("dotfiles/.zshrc.devsetup-tmp").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn finds_the_login_shell_of_a_user() {
        let passwd = "root:x:0:0:root:/root:/bin/bash\nme:x:1000:1000:Me:/home/me:/usr/bin/zsh\n";
        assert_eq!(login_shell_of(1000, passwd), Some("/usr/bin/zsh".to_string()));
        assert_eq!(login_shell_of(4242, passwd), None);
    }
}
