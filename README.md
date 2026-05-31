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
- максимальная ширина aggressive-сжатого изображения: `1600px`
- если исходная ширина `<= 1600px`, изображение не увеличивается
- сжатая версия всегда сохраняется как JPEG
- одновременно выполняется не больше `4` операций сжатия

Сервис использует два профиля сжатия:

- **safe**: для небольших изображений. Сначала выполняется lossless JPEG-оптимизация и очистка метаданных. Если файл уже хорошо оптимизирован, сервис может вернуть файл того же размера, чтобы не портить качество.
- **aggressive**: для больших изображений. Включается, если файл весит `>= 1 MB` или ширина больше `2400px`. Изображение уменьшается до ширины `1600px` и кодируется в JPEG quality `80`.

Для JPEG-файлов сервис пытается использовать внешние утилиты, если они установлены:

- `jpegtran -copy none -optimize -progressive` - lossless-оптимизация, удаление EXIF/GPS/preview-метаданных.
- `cjpeg -quality ... -optimize -progressive` - более эффективное JPEG-кодирование.

Если `jpegtran` или `cjpeg` недоступны, сервис использует встроенный encoder из Rust-библиотеки `image`.

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

Пример для большого изображения:

```json
{
  "original_size": 10174706,
  "compressed_size": 606569,
  "original_width": 10315,
  "original_height": 7049,
  "compressed_width": 1600,
  "compressed_height": 1093
}
```

Пример для уже оптимизированного небольшого JPEG:

```json
{
  "original_size": 289050,
  "compressed_size": 289050,
  "original_width": 1199,
  "original_height": 1058,
  "compressed_width": 1199,
  "compressed_height": 1058
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

Сервис слушает:

```text
127.0.0.1:3000
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

## Производительность и параллельность

Сжатие изображений - CPU- и memory-heavy операция. Например, JPEG на `9.7 MB` может занимать сотни мегабайт после декодирования в пиксели.

Чтобы одновременные загрузки не запускали слишком много тяжелой работы, сервис ограничивает параллельное сжатие:

```text
MAX_CONCURRENT_COMPRESSIONS = 4
```

Запросы сверх лимита ждут свободный слот. Декодирование, resize и JPEG-кодирование выполняются через `tokio::task::spawn_blocking`, чтобы не блокировать async runtime.

Для продакшена дополнительно стоит добавить:

- rate limiting;
- cleanup старых файлов в `uploads/`;
- очередь задач с `job_id`, если пользователи могут загружать много больших файлов одновременно;
- лимит не только по байтам, но и по megapixels/dimensions.

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
- Для большого файла размер сжатой версии стал меньше.
- Для маленького уже оптимизированного JPEG размер может остаться прежним.
- Качество визуально приемлемое.
