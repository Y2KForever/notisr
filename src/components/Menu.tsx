import { PaddingSpacing, ThemeToggle } from './ThemeToggle';

export const Menu = () => {
  return (
    <div className='flex flex-row float-right mr-2 mt-2'>
      <ThemeToggle hover={false} spacing={PaddingSpacing.SMALL} />
    </div>
  );
};
