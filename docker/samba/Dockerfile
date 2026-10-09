# Compartilhamento SMB só de leitura das gravações (serviço `samba` do compose.yaml).
# Imagem própria e mínima em vez de uma de terceiros: é só o smbd com o smb.conf abaixo.
FROM alpine:3.22
RUN apk add --no-cache samba-server samba-common-tools tini
COPY smb.conf /etc/samba/smb.conf
COPY entrypoint.sh /entrypoint.sh
RUN chmod 0755 /entrypoint.sh
EXPOSE 445
ENTRYPOINT ["/sbin/tini", "--", "/entrypoint.sh"]
