

# Acerca de `hq`

Mi asistente personal de IA y plataforma de productividad, compuesta por lo siguiente:
- Un servidor: API, archivos estáticos
- Interfaz de chat: bot de chat de IA, aplicación web progresiva (PWA)
- La CLI: ejecutar comandos desde la terminal
- IA: herramientas, agentes e integraciones

Este es un [software único](https://notes.alexkehayias.com/one-of-one-software/). Está diseñado desde cero para un solo usuario: yo. Aunque el código está disponible para referencia, no puedes copiarlo ni usarlo para ningún fin sin permiso (ni siquiera querrías hacerlo, ya que está diseñado para mí).

## Ejecutarlo

Inicializar el índice:

```
cargo run -- --init
```

Buscar notas:

```
cargo run -- query --term "testing" --vector
```

Indexar o reindexar:

```
cargo run -- index --all
```

Ejecutar el servidor:

```
cargo run -- serve --port 2222
```

Buscar notas usando el servidor:

```
curl http://localhost:2222/notes/search?query=test&include_similarity=true
```

Ejecutar un servidor de desarrollo que se recarga al cambiar archivos:

```
cargo install --locked watchexec-cli
./bin/watch.sh
```

## Docker

Construir la imagen:

```
docker build -t "hq:latest" .
```

Ejecutar un contenedor:

```
docker run -p 2222:2222 -d hq:latest
```

## Ejecutar en Dokku

1. En el servidor `dokku`, crear la aplicación
2. Crear un directorio para las claves de despliegue en `dokku` bajo `/var/lib/dokku/data/storage/hq/.ssh`
3. Generar un nuevo par de claves en el servidor `dokku` `ssh-keygen -q -t rsa -b 2048 -f "/var/lib/dokku/data/storage/hq/.ssh/notes_id_rsa" -N ""`
4. Almacenar la clave pública en el repositorio de GitHub (Settings -> Deploy Keys)
5. Generar known hosts de GitHub `sudo bash -c "ssh-keyscan -t rsa github.com >> /var/lib/dokku/data/storage/hq/.ssh/known_hosts"`
7. Crear directorio de datos para persistir índices entre despliegues `mkdir /var/lib/dokku/data/storage/hq/data && dokku storage:mount hq /var/lib/dokku/data/storage/hq:/root/data`
8. Crear directorio de habilidades `mkdir /var/lib/dokku/data/storage/hq/data/skills`
9. Crear directorio de espacio de trabajo para la memoria `mkdir /var/lib/dokku/data/storage/hq/data/workspace`
10. Generar un par de claves VAPID, clave privada con `openssl ecparam -genkey -name prime256v1 -out private_key.pem`, y clave pública con `openssl ec -in private_key.pem -pubout -outform DER|tail -c 65|base64|tr '/+' '_-'|tr -d '\n'` (eliminar el `=` final en la clave pública y codificarlo de forma estática en `index.js`)
11. Agregar las siguientes variables de entorno usando `dokku config:set hq {ENV_VAR}`:
- `HQ_NOTES_REPO_URL` con la URL del repositorio de GitHub con las notas
- `HQ_NOTES_DEPLOY_KEY_PATH` a `/root/.ssh`
- `HQ_STORAGE_PATH` para permitir que los índices persistan entre despliegues
- `HQ_VAPID_KEY_PATH` para notificaciones push
- `HQ_NOTE_SEARCH_API_URL` para la herramienta de búsqueda de notas por IA
- `HQ_GMAIL_CLIENT_ID` y `HQ_GMAIL_CLIENT_SECRET` para la API de Gmail
- `HQ_GOOGLE_SEARCH_API_KEY` y `HQ_GOOGLE_SEARCH_CX_ID` para la API de búsqueda de Google
- `HQ_LOCAL_LLM_HOST` para el nombre de host de la API de OpenAI (por defecto "https://api.openai.com" si no se establece)
- `HQ_CALENDAR_EMAIL` para nosotros en la preparación de reuniones
- `HQ_LOCAL_LLM_MODEL` para el modelo de OpenAI a utilizar (por defecto "gpt-4.1-mini" si no se establece)
- `HQ_SYSTEM_MESSAGE` para personalizar el mensaje base del sistema para las sesiones de chat
- `OPENAI_API_KEY` para la autenticación de la API de OpenAI (se ignora al usar un servidor LLM local)
- `DOKKU_DOCKERFILE_START_CMD` a `serve --host 0.0.0.0 --port 2222`
12. Localmente, agregar el remoto `git remote add dokku dokku@<dokku-host>:hq`
13. Enviar (push) para compilar e iniciar `git push dokku main`
14. Aumentar el tiempo de espera predeterminado del proxy `dokku nginx:set hq proxy-read-timeout 5m` y el tamaño máximo del cuerpo `dokku nginx:set lm-proxy client-max-body-size 10m`
15. Volver a desplegar la aplicación para que los cambios de `nginx` surtan efecto `dokku deploy hq`

## Consultas

Buscar en el índice utiliza AQL (Alex Query Language), que es aproximadamente la sintaxis de `orgql` con algunas personalizaciones.

| **Tipo**            | **Ejemplo**                | **Notas**                                                      |
|---------------------|----------------------------|----------------------------------------------------------------|
| Término con campo   | `title:rust`               | Término único                                                  |
| Frasa               | `title:"rust programming"` | Término entre comillas                                         |
| Múltiples valores   | `title:rust,python`        | Los valores múltiples separados por coma se combinan con Y lógico |
| Término por defecto | `hello world`              | Por defecto busca en el campo cuerpo y título                  |
| Negación            | `-title:rust`              | Niega cualquier término                                        |
| Rango               | `date:>2025-01-01`         | Operaciones soportadas `>`, `>=`, `<`, `<=`                    |
