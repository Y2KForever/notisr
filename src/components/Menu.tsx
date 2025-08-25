import { PaddingSpacing, ThemeToggle } from './ThemeToggle';

export const Menu = () => {
  return (
    <div className="w-full">
      <ThemeToggle hover={false} spacing={PaddingSpacing.SMALL} />
    </div>
  );
};
