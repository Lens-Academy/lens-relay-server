FROM node:22-alpine AS builder
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci
COPY . .
RUN npx vite build

FROM node:22-alpine
WORKDIR /app
ENV NODE_ENV=production
COPY package.json package-lock.json ./
RUN npm ci --omit=dev
COPY --from=builder /app/dist ./dist
COPY server/prod-server.ts server/auth-middleware.ts server/share-token.ts ./server/
COPY shared/ ./shared/
EXPOSE 3000
CMD ["npx", "tsx", "server/prod-server.ts"]
