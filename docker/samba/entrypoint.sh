#!/bin/sh
# Cria o usuário do compartilhamento com o mesmo UID do dono das gravações e a senha do segredo
# `smb_password` (nunca na linha de comando nem no log), e sobe o smbd em primeiro plano.
set -eu

user="${SMB_USER:-rrv}"
uid="${SMB_UID:-1000}"
gid="${SMB_GID:-1000}"
secret="/run/secrets/smb_password"

case "$user" in
    *[!a-z0-9_-]* | "") echo "samba: SMB_USER inválido: use letras minúsculas, dígitos, _ ou -" >&2; exit 1 ;;
esac
if [ ! -s "$secret" ]; then
    echo "samba: falta a senha em secrets/smb_password (veja o compose.yaml)" >&2
    exit 1
fi

getent group "$gid" >/dev/null || addgroup -g "$gid" "$user"
group="$(getent group "$gid" | cut -d: -f1)"
id "$user" >/dev/null 2>&1 || adduser -D -H -s /sbin/nologin -u "$uid" -G "$group" "$user"
sed -i "s/@SMB_USER@/$user/g" /etc/samba/smb.conf

pass="$(head -n1 "$secret" | tr -d '\r\n')"
printf '%s\n%s\n' "$pass" "$pass" | smbpasswd -a -s "$user" >/dev/null
unset pass

echo "samba: \\\\<este-host>\\gravacoes (usuário $user, só leitura)"
exec smbd --foreground --no-process-group --debug-stdout
