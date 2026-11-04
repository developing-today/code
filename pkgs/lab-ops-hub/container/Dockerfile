FROM python:3.13-alpine
WORKDIR /app
COPY requirements.txt .
RUN python3 -m pip install --no-cache-dir -r requirements.txt
COPY app.py .
EXPOSE 8080
CMD ["python3", "app.py"]
