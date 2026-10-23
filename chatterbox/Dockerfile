FROM python:3.11-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends libgomp1 \
    && rm -rf /var/lib/apt/lists/*

# Keep these pins in step with the header of server.py.
RUN pip install --no-cache-dir \
    "chatterbox-tts==0.1.7" \
    "setuptools<80" \
    "fastapi==0.161.1" \
    "uvicorn==0.54.0"

COPY server.py /app/server.py

CMD ["python", "/app/server.py", "--host", "0.0.0.0"]
