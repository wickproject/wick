FROM ubuntu:24.04

# Install Node.js 20
RUN apt-get update \
    && apt-get install -y --no-install-recommends curl ca-certificates gnupg \
    && curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*

# Install Wick binary + libcronet.so from GitHub release
ARG WICK_VERSION=0.8.0
ARG WICK_SHA256=acc08ade37231376121b50adc588641cb11cd3472373ad53a445c8dd81a13b6e

RUN cd /tmp \
    && curl -fsSL "https://github.com/wickproject/wick/releases/download/v${WICK_VERSION}/wick-linux-amd64.tar.gz" -o wick.tar.gz \
    && echo "${WICK_SHA256}  wick.tar.gz" | sha256sum -c - \
    && tar xzf wick.tar.gz \
    && mv wick /usr/local/bin/wick \
    && mv libcronet.so /usr/local/lib/libcronet.so \
    && chmod +x /usr/local/bin/wick \
    && ldconfig \
    && rm wick.tar.gz

# Verify wick runs
RUN wick version

WORKDIR /app
COPY package.json .
RUN npm install --production
COPY . .

CMD ["node", "src/main.js"]
