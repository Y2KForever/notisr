import { util } from '@aws-appsync/utils';

export function request(ctx) {
  const { broadcaster_id, ...values } = ctx.arguments;

  return {
    operation: 'PutItem',
    key: {
      broadcaster_id: { S: broadcaster_id },
    },
    attributeValues: util.dynamodb.toMapValues(values),
  };
}

export function response(ctx) {
  if (ctx.error) {
    util.error(ctx.error.message, ctx.error.type);
  }
  return ctx.result;
}
