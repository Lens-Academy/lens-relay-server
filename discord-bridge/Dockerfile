FROM node:22-alpine
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci
COPY src/ ./src/
COPY tsconfig.json ./
ENV NODE_ENV=production
ENV DISCORD_BRIDGE_PORT=8091
EXPOSE 8091
CMD ["npx", "tsx", "src/index.ts"]
