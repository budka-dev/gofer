# Контракт HTTP-эмбеддера

gofer **не считает эмбеддинги локально**. Вместо этого он POSTит батчи текстов на внешний HTTP-сервис. Этот документ описывает спецификацию, которой должен соответствовать такой сервис, чтобы gofer мог с ним работать.

Реализация в gofer: `src/indexer/embedder.rs::Embedder`. Точка конфигурации: `[embedding]` в `.gofer/config.toml` (см. [config-reference.md](config-reference.md#embedding)).

## Зачем внешний сервис

- Развязывает gofer и модель — можно крутить любой эмбеддер (BGE, Nomic, OpenAI, BAAI, собственный), не пересобирая gofer.
- Снимает с gofer'а зависимость от ONNX/Candle/ort/CUDA — бинарь остаётся легковесным.
- Позволяет шарить эмбеддер между несколькими инстансами gofer на разных проектах.

Цена — нужно где-то крутить этот сервис. Минимальная реализация на FastAPI + sentence-transformers занимает 20 строк (пример в конце).

## Запрос

**Метод:** `POST`
**Endpoint:** значение `[embedding].external_url` из конфига. По умолчанию `http://127.0.0.1:8080/embed/`. Важен trailing slash, если у тебя сервер строгий.
**Content-Type:** `application/json`
**Headers:** `x-api-key: <value>`, если задан `[embedding].external_api_key`.

**Тело:**

```json
{
  "inputs": ["text 1", "text 2", "..."],
  "input": ["text 1", "text 2", "..."],
  "model": "nomic-embed-text-v1.5"
}
```

Замечания:

- gofer дублирует список текстов под двумя ключами `inputs` и `input` — для совместимости с TEI (Text Embeddings Inference от Hugging Face) и OpenAI-style API одновременно. Сервис может читать любой из них.
- `model` присутствует только если задан `[embedding].external_model`.
- Размер батча контролируется gofer'ом, верхняя граница — `[embedding].batch_size` (по умолчанию 32).
- HTTP таймаут на клиенте — 60 с (`reqwest::Client::builder().timeout(Duration::from_secs(60))`).

## Ответ

gofer принимает три разных формата ответа — это сделано для совместимости с готовыми сервисами. Сервер может выбрать любой.

### 1. Прямой массив массивов (минимальный)

```json
[
  [0.123, -0.045, 0.876, ...],
  [0.231, 0.667, -0.012, ...]
]
```

### 2. Объект с ключом `embeddings` (TEI-стиль)

```json
{
  "embeddings": [
    [0.123, -0.045, ...],
    [0.231, 0.667, ...]
  ]
}
```

### 3. OpenAI-стиль

```json
{
  "data": [
    { "embedding": [0.123, -0.045, ...], "index": 0 },
    { "embedding": [0.231, 0.667, ...], "index": 1 }
  ],
  "model": "...",
  "usage": { "total_tokens": 123 }
}
```

Любые дополнительные поля (`model`, `usage`, `index`) игнорируются.

### Требования к векторам

- Каждый вектор — массив `f64` (внутри gofer кастится в `f32`).
- Длина вектора **должна** совпадать с `[embedding].dimensions` из конфига. LanceDB ловит несоответствие схеме и отбрасывает batch.
- Порядок ответов должен совпадать с порядком входных текстов.

## Ошибки

- HTTP-статус не `2xx` → gofer трактует как сбой. Тело ответа попадает в лог (`External embedder error ({status}): {body}`).
- 5 подряд ошибок → `embedding_circuit` размыкается на 30 с, gofer перестаёт ходить в эмбеддер до cooldown.
- Невалидный JSON или незнакомая структура → `Unknown response format`. Тоже считается сбоем.

## Минимальный пример сервера (FastAPI)

```python
# requirements: fastapi uvicorn sentence-transformers
from fastapi import FastAPI
from pydantic import BaseModel
from sentence_transformers import SentenceTransformer

app = FastAPI()
model = SentenceTransformer("nomic-ai/nomic-embed-text-v1.5", trust_remote_code=True)

class EmbedRequest(BaseModel):
    inputs: list[str] | None = None
    input: list[str] | None = None
    model: str | None = None

@app.post("/embed/")
def embed(req: EmbedRequest):
    texts = req.inputs or req.input or []
    vectors = model.encode(texts, normalize_embeddings=True).tolist()
    return {"embeddings": vectors}
```

Запуск:

```bash
uvicorn server:app --host 127.0.0.1 --port 8080
```

В `.gofer/config.toml`:

```toml
[embedding]
external_url = "http://127.0.0.1:8080/embed/"
external_model = "nomic-embed-text-v1.5"
dimensions = 768       # Размер для nomic-embed-text-v1.5
pool_size = 4
batch_size = 32
```

Перед первой индексацией убедись, что сервис отвечает:

```bash
curl -X POST http://127.0.0.1:8080/embed/ \
  -H 'content-type: application/json' \
  -d '{"inputs":["hello world"]}' | jq '.embeddings[0] | length'
# должно вывести dimensions (например, 768)
```

## Совместимость с готовыми сервисами

| Сервис | Совместимость | Замечания |
|---|---|---|
| **HuggingFace TEI** | да | Формат `{"embeddings": [...]}`, `--port 8080 --auto-truncate`. |
| **OpenAI `/embeddings`** | да | `external_url = "https://api.openai.com/v1/embeddings"`, `external_api_key` = OPENAI_API_KEY, gofer шлёт `x-api-key` — для OpenAI понадобится reverse-proxy с переписыванием на `Authorization: Bearer`. |
| **Ollama embeddings** | нет | Возвращает поле `embedding` (single), не batch. Нужен адаптер. |
| **Custom (FastAPI, Flask, Express)** | да | Минимум — формат №1 или №2 из секции «Ответ». |

## Производительность

- Латентность запроса напрямую попадает в время индексации (одна из самых горячих стадий pipeline).
- Если эмбеддер крутится локально на GPU и поддерживает батчи — `batch_size = 64..128` обычно даёт прирост.
- Pool в gofer (`pool_size = 4`) — это количество **HTTP-клиентов**, не моделей. Если у эмбеддера один GPU и нет внутренней очереди, толку от больших значений нет.
- Кеш эмбеддингов (`chunk_cache` в SQLite) сохраняет результат по `content_hash`. При инкрементальной индексации повторно не считается. Ключ инвалидируется при смене `external_model` (`cache_version_key()`).

## Where to look in code

- `src/indexer/embedder.rs::Embedder::new` — построение запроса.
- `src/indexer/embedder.rs::Embedder::embed` — разбор трёх форматов ответа.
- `src/indexer/embedder.rs::EmbedderPool` — пул клиентов и семафор.
- `src/indexer/pipeline.rs::embedder_stage` — кто и когда зовёт эмбеддер.
