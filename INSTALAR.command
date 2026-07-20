#!/usr/bin/env bash
# INSTALAR.command — Black Wall Mod System (bwms) 0.1.3 — double-click in Finder.
# Finds Cyberpunk 2077 (or you drag the game icon/folder), removes quarantine, installs.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"

# 0) remove macOS "quarantine" from this kit (anything from the internet gets it;
#    without removing it macOS blocks the scripts and the dylib).
xattr -dr com.apple.quarantine "$HERE" 2>/dev/null || true

# Cleans a path the user dragged in: Terminal escapes spaces as "\ " and may add
# quotes / a trailing space. We strip the backslash-escapes, quotes and edge spaces.
clean_path(){
  local p="$1"
  p="${p//\\/}"               # remove backslash escapes (drag turns " " into "\ ")
  p="${p#\"}"; p="${p%\"}"    # strip surrounding double quotes
  p="${p#\'}"; p="${p%\'}"    # strip surrounding single quotes
  p="${p#"${p%%[![:space:]]*}"}"   # trim leading whitespace
  p="${p%"${p##*[![:space:]]}"}"   # trim trailing whitespace
  printf '%s' "$p"
}

is_game(){ [ -f "$1/Cyberpunk2077.app/Contents/MacOS/Cyberpunk2077" ]; }

# Resolves whatever the user dragged — the game folder, the "Cyberpunk2077" app
# icon, or a file inside it — to the actual game folder, by climbing up until it
# finds the one that holds Cyberpunk2077.app.
resolve_game(){
  local cur; cur="$(clean_path "$1")"; cur="${cur%/}"
  local i
  for i in 1 2 3 4 5 6; do
    [ -n "$cur" ] || break
    if is_game "$cur"; then printf '%s' "$cur"; return 0; fi
    cur="$(dirname "$cur")"
    [ "$cur" = "/" ] && break
  done
  return 1
}

# --- language pick ---------------------------------------------------------
printf '\n  Black Wall Mod System (bwms) 0.1.3\n\n'
printf '  Language — type a number, then press Enter:\n'
printf '    1) English  (default)\n'
printf '    2) Português\n'
printf '    3) 简体中文  (Simplified Chinese)\n'
printf '  > '
read -r LSEL
case "$LSEL" in
  2) LC=pt ;;
  3) LC=zh ;;
  *) LC=en ;;
esac

case "$LC" in
  pt)
    M_TITLE="Black Wall Mod System (bwms) 0.1.3 — beta"
    M_NOTFOUND="Não achei o Cyberpunk 2077 automaticamente."
    M_DRAG="ARRASTE para esta janela o ícone 'Cyberpunk2077' do jogo (ou a pasta 'Cyberpunk 2077') e tecle Enter:"
    M_INVALID="Não reconheci como Cyberpunk 2077:"
    M_NEEDFOLDER="(arraste o ícone 'Cyberpunk2077' ou a pasta do jogo)."
    M_CLOSE="Tecle Enter para fechar."
    M_FOUND="Jogo encontrado:"
    M_DONE="TUDO PRONTO. Abra o Cyberpunk 2077 pela sua loja (Steam / GOG / Epic)."
    M_INGAME='No jogo: a tecla  `  abre o console; ESC > Configurações > Mods tem os cheats.'
    M_PROBLEM="A instalação encontrou um problema (código"
    M_SEEABOVE="Veja as mensagens acima."
    M_CLOSEWIN="Tecle Enter para fechar esta janela."
    ;;
  zh)
    M_TITLE="Black Wall Mod System (bwms) 0.1.3 — 测试版"
    M_NOTFOUND="未能自动找到 Cyberpunk 2077。"
    M_DRAG="请把游戏的 'Cyberpunk2077' 图标（或 'Cyberpunk 2077' 文件夹）拖到此窗口，然后按回车："
    M_INVALID="无法识别为 Cyberpunk 2077："
    M_NEEDFOLDER="（请拖入 'Cyberpunk2077' 图标或游戏文件夹）。"
    M_CLOSE="按回车键关闭。"
    M_FOUND="已找到游戏："
    M_DONE="全部完成。请通过你的商店（Steam / GOG / Epic）启动 Cyberpunk 2077。"
    M_INGAME='游戏中：按  `  键打开控制台；ESC > 设置 > Mods 里有作弊选项。'
    M_PROBLEM="安装遇到问题（代码"
    M_SEEABOVE="请查看上方的信息。"
    M_CLOSEWIN="按回车键关闭此窗口。"
    ;;
  *)
    M_TITLE="Black Wall Mod System (bwms) 0.1.3 — beta"
    M_NOTFOUND="Couldn't find Cyberpunk 2077 automatically."
    M_DRAG="DRAG the game's 'Cyberpunk2077' icon (or the 'Cyberpunk 2077' folder) onto this window and press Enter:"
    M_INVALID="Didn't recognize that as Cyberpunk 2077:"
    M_NEEDFOLDER="(drag the 'Cyberpunk2077' icon or the game folder)."
    M_CLOSE="Press Enter to close."
    M_FOUND="Game found:"
    M_DONE="ALL DONE. Launch Cyberpunk 2077 from your store (Steam / GOG / Epic)."
    M_INGAME='In game: the  `  key opens the console; ESC > Settings > Mods has the cheats.'
    M_PROBLEM="The install hit a problem (code"
    M_SEEABOVE="See the messages above."
    M_CLOSEWIN="Press Enter to close this window."
    ;;
esac

echo "============================================"
echo "  $M_TITLE"
echo "============================================"
echo

# 1) find the Cyberpunk 2077 folder (the one with Cyberpunk2077.app)
GAME=""
for c in \
  "$HOME/Library/Application Support/Steam/steamapps/common/Cyberpunk 2077" \
  "$HOME/GOG Games/Cyberpunk 2077" \
  "/Applications/Cyberpunk 2077" \
  "/Users/Shared/Epic Games/Cyberpunk 2077" \
  "$HOME/Library/Application Support/Epic/Cyberpunk 2077" \
  "$(cd "$HERE/.." 2>/dev/null && pwd)/Cyberpunk 2077" \
  "$(cd "$HERE/../.." 2>/dev/null && pwd)" ; do
  if is_game "$c"; then GAME="$c"; break; fi
done

if [ -z "$GAME" ]; then
  echo "$M_NOTFOUND"
  echo "$M_DRAG"
  read -r DROPPED
  GAME="$(resolve_game "$DROPPED")" || GAME=""
fi
if [ -z "$GAME" ] || ! is_game "$GAME"; then
  echo; echo "$M_INVALID ${DROPPED:-$GAME}"; echo "$M_NEEDFOLDER"
  read -r -p "$M_CLOSE "; exit 1
fi
echo "$M_FOUND $GAME"; echo

# 2) run the install engine with the dylib that ships IN THIS package
export BWMS_DYLIB_SRC="$HERE/engine/libcp77_console.dylib"
export BWMS_LANG="$LC"
bash "$HERE/engine/bwms-install.sh" "$GAME"
EC=$?
echo
if [ $EC -eq 0 ]; then
  echo "$M_DONE"
  echo "$M_INGAME"
else
  echo "$M_PROBLEM $EC). $M_SEEABOVE"
fi
read -r -p "$M_CLOSEWIN "
