FROM python:3.12-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    git \
    openssh-client \
    rsync \
    curl \
    netcat-openbsd \
    xz-utils \
    nodejs  \
    npm  \
    && rm -rf /var/lib/apt/lists/*

# for github.com ssh access
COPY dot.ssh/* /root/.ssh/

RUN mkdir -p /scripts
COPY check-push.sh /scripts
COPY prod2latest.sh /scripts
COPY cleanup-archives.sh /scripts

WORKDIR /webhook
COPY hook.py /webhook
COPY *.whl /webhook
RUN pip install /webhook/*.whl

CMD /webhook/hook.py