# Image Compression Service

MVP Rust-сервиса для приема изображения, сохранения оригинала и создания сжатой JPEG-версии.

## API

Базовый адрес локального сервиса:

```text
http://localhost:3000
```

### GET /health

Проверяет, что сервис запущен.

Пример запроса:

```bash
curl http://localhost:3000/health
```

Успешный ответ:

```json
{
  "status": "ok"
}
```

HTTP статус:

```text
200 OK
```

### POST /images/compress

Принимает изображение, сохраняет оригинал, создает сжатую JPEG-версию и возвращает информацию о результате.

Формат запроса:

```text
multipart/form-data
```

Поле файла:

```text
file
```

Ограничения:

- максимальный размер файла: `32 MB`
- максимальная ширина сжатого изображения: `1600px`
- если исходная ширина `<= 1600px`, изображение не увеличивается
- сжатая версия всегда сохраняется как JPEG
- JPEG quality: `80`

Пример запроса:

```bash
curl -X POST http://localhost:3000/images/compress \
  -F "file=@test-images/photo.jpg"
```

Успешный ответ:

```json
{
  "original_file_name": "89c53fa9-7719-4425-9bfc-e5bef02db77a_original.jpg",
  "original_path": "uploads/originals/89c53fa9-7719-4425-9bfc-e5bef02db77a_original.jpg",
  "compressed_file_name": "89c53fa9-7719-4425-9bfc-e5bef02db77a_compressed.jpg",
  "compressed_path": "uploads/compressed/89c53fa9-7719-4425-9bfc-e5bef02db77a_compressed.jpg",
  "original_size": 10174706,
  "compressed_size": 842988,
  "original_width": 10315,
  "original_height": 7049,
  "compressed_width": 1600,
  "compressed_height": 1093
}
```

Поля ответа:

- `original_file_name` - имя сохраненного оригинального файла.
- `original_path` - относительный путь к оригинальному файлу.
- `compressed_file_name` - имя сохраненного сжатого файла.
- `compressed_path` - относительный путь к сжатому файлу.
- `original_size` - размер оригинального файла в байтах.
- `compressed_size` - размер сжатого файла в байтах.
- `original_width` - ширина оригинального изображения в пикселях.
- `original_height` - высота оригинального изображения в пикселях.
- `compressed_width` - ширина сжатого изображения в пикселях.
- `compressed_height` - высота сжатого изображения в пикселях.

HTTP статус:

```text
200 OK
```

### Ошибки

Все ошибки возвращаются в едином JSON-формате:

```json
{
  "error": "Invalid image file"
}
```

Возможные ошибки:

| HTTP статус | error | Когда возникает |
| --- | --- | --- |
| `400 Bad Request` | `No file provided` | В multipart-запросе нет поля `file`. |
| `400 Bad Request` | `Invalid image file` | Переданный файл не удалось прочитать как изображение. |
| `413 Payload Too Large` | `File too large` | Размер файла больше `32 MB`. |
| `500 Internal Server Error` | `Failed to create uploads directory` | Не удалось создать папку для сохранения файлов. |
| `500 Internal Server Error` | `Failed to save original image` | Не удалось сохранить оригинальный файл. |
| `500 Internal Server Error` | `Failed to save compressed image` | Не удалось сохранить сжатый файл. |
| `500 Internal Server Error` | `Internal server error` | Внутренняя ошибка обработки запроса. |

## Запуск

```bash
cargo run
```

## Health check

```bash
curl http://localhost:3000/health
```

## Сжатие изображения

```bash
curl -X POST http://localhost:3000/images/compress \
  -F "file=@test.jpg"
```

## Где искать оригиналы

```text
uploads/originals/
```

## Где искать сжатые изображения

```text
uploads/compressed/
```

## Локальная ручная проверка

Положите тестовую фотографию в:

```text
test-images/photo.jpg
```

Запустите сервис:

```bash
cargo run
```

В другом терминале отправьте изображение:

```bash
curl -X POST http://localhost:3000/images/compress \
  -F "file=@test-images/photo.jpg"
```

После запроса проверьте:

- JSON-ответ.
- Появился ли оригинал в `uploads/originals/`.
- Появилась ли сжатая версия в `uploads/compressed/`.
- Открываются ли оба файла.
- Размер сжатой версии стал меньше.
- Качество визуально приемлемое.
