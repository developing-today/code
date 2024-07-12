FROM ubuntu

ENV DEV false

RUN apt-get update && apt-get install ucspi-tcp

EXPOSE 3333

COPY . /app

CMD [ "/app/start.sh" ]
