import * as ddb from '@aws-appsync/utils/dynamodb';

export function request(ctx) {
  return ddb.get({ key: { broadcaster_id: ctx.args.broadcaster_id } });
}

export const response = (ctx) => ctx.result;
