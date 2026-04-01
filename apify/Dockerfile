FROM node:20-slim

# Install Wick binary + libcronet.so from GitHub release
ARG WICK_VERSION=0.7.0
ADD https://github.com/wickproject/wick/releases/download/v${WICK_VERSION}/wick-linux-amd64.tar.gz /tmp/wick.tar.gz
RUN cd /tmp && tar xzf wick.tar.gz \
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
