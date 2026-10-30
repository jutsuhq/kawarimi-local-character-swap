# Kawarimi: план

Kawarimi (変わり身の術, техника подмены) — замена человека в видео на персонажа с картинки, локально на Windows и
Linux. Подготовка, промпт и рендер работают на машине пользователя, без облака и оплаты за ролик. Пайплайн и дизайн
перенесены из Swapjutsu (macOS), модель рендера взаимозаменяемая; первая — Wan 2.2 Animate.

- Репозиторий: `jutsuhq/kawarimi-local-character-swap`.
- Описание: "Local AI character swap for video on Windows and Linux: replace a person in any clip with a character
  from one image. Runs entirely on your NVIDIA GPU. Open source."
- Topics: `character-swap`, `video-to-video`, `wan-animate`, `comfyui`, `tauri`, `local-ai`, `offline`, `edgetam`.
- Похожие проекты: `opusforge/gorilla-studio`, `andrewloya/genjutsu-studio` (тот же питч),
  `rik-python/AI-CharacterSwap-Workflow` (workflow без приложения).

## Решения

| Вопрос | Решение |
|---|---|
| Платформы | Windows и Linux, только NVIDIA (CUDA) |
| Оболочка | Tauri 2, ядро на Rust, фронтенд React + TypeScript + Vite + Tailwind v4 |
| Подготовка | Rust + ONNX Runtime (крейт `ort`, CUDA EP), готовые ONNX-конверсии |
| Промпт | LLM — опция, выключаемая в Settings; без неё — шаблон |
| Рендер | Wan 2.2 Animate на локальном ComfyUI; ComfyUI ставит приложение, свой — опцией |

## Чем отличается от Swapjutsu

Control-видео с глубиной, скрытое лицо и вокал +3 полутона в Swapjutsu нужны только облачному Seedance. Локальному
Wan 2.2 Animate нужны исходное видео, маска заменяемого человека, картинка персонажа и промпт; позу он извлекает сам
(DWPose в ComfyUI). Глубина и обработка звука в первую версию не входят. Итог — результат Wan + исходная дорожка.

## Этапы и модели

Все веса скачиваются при первом использовании по зафиксированной ревизии и проверяются по SHA-256 до загрузки.

| # | Этап | Модель (основная → запасная) | Лицензия | Статус |
|---|---|---|---|---|
| 1 | Нормализация | ffmpeg (sidecar), 16 fps и размер под Wan | LGPL-сборка | — |
| 2 | Шоты | TransNetV2: `edgetools/transnetv2@4b2cef0` → `elya5/transnetv2` | MIT | parity с оригиналом не проверена |
| 3 | Люди (рамки) | RT-DETRv2-R18: `onnx-community/rtdetr_v2_r18vd-ONNX@936f90b` → RTMDet-Ins-S с масками | Apache-2.0 | на CUDA EP не проверено |
| 4 | Лица | YuNet: `opencv/face_detection_yunet@3cc26e7` (вход 640²) → BlazeFace | MIT | на ORT не проверено |
| 5 | Трекинг | EdgeTAM video: `jax-image-tools/edgetam-video-onnx@8ca3d3e` → SAM 2.1 tiny video: `square-zero-labs/sam2.1-tiny-video-onnx@3b2984d` | Apache-2.0 | см. фазу 1 |
| 6 | Идентичность | цвет HSV внутри маски + average linkage, логика Swapjutsu | — | порт в Rust |
| 7 | Цели | больше всего на экране / выбранные | — | порт в Rust |
| 8 | Вырезка персонажа | BiRefNet lite: `onnx-community/BiRefNet_lite-ONNX@de15b22` → BiRefNet general | MIT | rembg использует тот же граф |
| 9 | Промпт (опция) | `llama-server` + Qwen3.5-9B Q4_K_M (`unsloth/Qwen3.5-9B-GGUF`) → Qwen3-VL-8B (официальный GGUF) | MIT / Apache-2.0 | — |
| 10 | Рендер | Wan 2.2 Animate в ComfyUI, окна по 77 кадров, как в Swapjutsu | Apache-2.0 | — |
| 11 | Финиш | ffmpeg: результат + исходный звук | — | — |

Отсеяно по лицензиям: Ultralytics YOLO и YOLOv10 из официального препроцессинга Wan (AGPL), SCRFD/InsightFace и
RMBG-2.0 (некоммерческие), Depth Anything V2 Base/Large (CC-BY-NC).

Трекинг — главный риск. Готовый EdgeTAM video ONNX один: 0 скачиваний, исходники экспорта недоступны, проверялся
только в WebGPU. Зато в нём динамические оси памяти, без заполнения пустых слотов нулями (проблема, которую
Swapjutsu так и не решил). Запасной SAM 2.1 tiny проверен в продакшене (IoU 0.9967 к PyTorch), но банк в нём
заполнен до 7 + 16 слотов. Box-промпт (углы с метками 2/3) не документирован ни в одном экспорте — проверить.

## LLM

`llama-server` (llama.cpp) как sidecar, отдельной загрузкой (~0.5 GB с CUDA runtime), сборка зафиксирована.
Единственный вариант с готовыми CUDA-сборками для Windows и Linux. Он же даёт OpenAI-совместимый API, поэтому
локальный режим и режим «свой API-ключ» — один клиент. Перед рендером процесс завершается и освобождает всю
видеопамять. Отвергнуты: `llama-cpp-2` (нет готовых сборок, CUDA-тулчейн на каждой машине, падение C++ роняет
приложение), mistral.rs (сборка с CUDA на Windows сломана), candle (нет Qwen-VL).

## Экраны и дизайн

Как в Swapjutsu: сайдбар (Create, Jobs, Library, Settings); в Create — лента клипов и рабочая область по шагам
Video → Prep → Characters → Render; плеер с вкладками Original / Mask / Review / Draft. Вместо цены — оценка времени
рендера и видеопамяти. Токены из `Theme.swift`: фон `#120D16`, сайдбар `#19121E`, текст `#F0EFED`, акцент `#F2FF59`,
ink `#211927`, радиусы 8/10/12/14/20, Barlow Condensed ExtraBold для заголовков (OFL), только тёмная тема, без
градиентов.

## Из ревью Swapjutsu — сразу в требования

- Ничего не качать без явного действия пользователя; никаких сторонних запросов без согласия.
- Задание рендера — явная машина состояний с терминальными статусами и восстановлением после перезапуска
  приложения и ComfyUI; результат не теряется.
- Агенты (если будут) — лимит на число заданий и время GPU; учёт вне папки, куда может писать агент.

## Фазы

| Фаза | Результат | Проверка |
|---|---|---|
| 0 | Каркас: Tauri, тема, сайдбар, Settings, ffmpeg-sidecar, загрузчик моделей с SHA-256 | сборка на Windows и Linux в CI |
| 1 | Трекинг: оба ONNX-трекера на CUDA EP из Rust | IoU к PyTorch EdgeTAM на bedroom-видео (как `edgetam-coreml/parity.py`), скорость на кадр; выбрать один |
| 2 | CLI `kawarimi-prep`: нормализация → шоты → люди → трекинг → `mask.mp4` | IoU масок на разметке `benchmarks/data/annotations` Swapjutsu |
| 3 | Идентичность через склейки, цели, лица, вырезка персонажа | те же клипы, сравнение с манифестами Swapjutsu |
| 4 | Менеджер ComfyUI + Wan 2.2 Animate + окна + финиш | рендер 5-секундного клипа от начала до конца |
| 5 | LLM-опция: `llama-server`, промпт из кадров и персонажа | промпты на клипах из примеров; выключение освобождает VRAM |
| 6 | Весь интерфейс: Create, Jobs, Library | сценарий от загрузки клипа до готового видео |
| 7 | Упаковка: NSIS/MSI, AppImage/deb, подпись | установка на чистую машину |

Фазы 0 и 1 независимы и могут идти параллельно.

## Открытые вопросы

- Сколько видеопамяти нужно Wan 2.2 Animate 14B в fp8/GGUF — проверить; ориентир 16–24 GB.
- MCP-сервер для агентов — в первой версии или позже.
- Лицензия проекта: MIT, как у Swapjutsu (дизайн и логику переносим с сохранением уведомления).
