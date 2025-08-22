import * as ddb from '@aws-appsync/utils/dynamodb';

export function request(ctx) {
  const item = { ...ctx.arguments };
  const key = { broadcaster_id: ctx.args.broadcaster_id };
  return ddb.put({ key, item });
}

export function response(ctx) {
  return ctx.result;
}
